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
//! (0x433bc5). [`World::update_rooms`] is the room update (0x466c3c): it
//! counts each room's tiles and the objects standing on them, works out
//! the room's light and score, and writes the lot's indoor area to global
//! 27. [`Tuning`] holds the constants and the arithmetic of the scores.
//!
//! Ported from the old prototype (legacy/crates/sims-sim/src/lot_rooms.rs,
//! lot_flags.rs, lot_walls.rs, lot_geometry.rs, `room_outdoors` in
//! spatial.rs, and lot_environment.rs with its `manager.rs` and
//! `objects.rs`), which compared each piece with the original's code run
//! alone. For the Goth house the results were also compared with a
//! trace of the original loading it; see `docs/decisions.md`.

use crate::load::{FLOORS, WALLS};
use crate::{FULL_SIZE, World};
use serde::{Deserialize, Serialize};
use sims_core::{Ext, F32};
use sims_formats::lot::Array;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// The room of an object that is in no room, and the map's mark for a tile
/// that a diagonal wall splits between two rooms.
pub const NO_ROOM: i16 = -5;

/// The map's value for a tile no room took: every tile outside the lot's
/// size.
pub const UNSEEN: i16 = -1;

/// Global 27: the area of the lot's indoor rooms, in tiles.
const INDOOR_AREA: usize = 27;

/// Global 4: the time of day. 0 is day, 1 dawn, 2 night and 3 dusk.
const TIME_OF_DAY: usize = 4;

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
    /// The room update was told of an object the lot does not have, or
    /// one that lacks a data word the update reads.
    Standing(u16),
    /// The objects on a tile could not be listed.
    Tile { tile: [u8; 3], why: String },
    /// A dark indoor room with people in it and no lights. Whether it is
    /// lit for them depends on the room knowing it is occupied
    /// (0x468802), which is not ported.
    Occupancy(i16),
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
            Self::Standing(object) => write!(
                f,
                "the room update was told of object {object}, which the lot lacks or which \
                 lacks a data word the update reads"
            ),
            Self::Tile { tile, why } => write!(
                f,
                "the objects on tile {}, {} of level {} could not be listed: {why}",
                tile[0], tile[1], tile[2]
            ),
            Self::Occupancy(room) => write!(
                f,
                "room {room} is dark, has no lights and has people in it; whether it is lit \
                 for them depends on the room knowing it is occupied, which is not ported"
            ),
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
    /// Which levels have a room waiting to be counted: bit 0 for the
    /// ground level and bit 1 for the upper level (the original's room
    /// manager, word 0x34). [`World::mark_room`] sets a bit and
    /// [`World::update_rooms`] clears the bits of the levels it counted.
    /// A room can stay marked with its level's bit clear; see
    /// [`World::update_rooms`].
    pub pending: u8,
    /// The sum of the outdoor rooms' scores, and the lot's outdoor score
    /// worked out from it (the room manager's words 0x40 and 0x3c). Zero
    /// until the rooms are first counted.
    pub outdoor_sum: F32,
    pub outdoor_score: F32,
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
    /// The mark that the room is not counted: set when the room is made
    /// and by [`World::mark_room`], cleared when [`World::update_rooms`]
    /// counts the room, and set again at once if that changed the room's
    /// light.
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
    /// The objects counted in the room: those standing on its listed
    /// tiles, a tile listed twice counting twice.
    pub objects: i32,
    /// The sum of data word 51 of the windows that are light sources, and
    /// of the other light sources.
    pub window_light: i32,
    pub lamp_light: i32,
    /// The sum of the objects' data word 39. What it is for is not known.
    pub word_39: i32,
    /// The sum of the objects' data word 7: how much each adds to or
    /// takes from the room's score. Outdoors only what takes away counts.
    pub impact: i32,
    /// The objects whose data word 7 is not negative, plus the listed
    /// tiles that are pool.
    pub good: i32,
    /// The objects whose data word 59 is 4, and those where it is 5 to 7.
    pub category_4: i32,
    pub category_5_to_7: i32,
    /// The light sources that stand on the lot and are not windows.
    pub lights: i32,
    /// The people.
    pub people: i32,
    /// The room's score, -100 to 100 indoors, zero until it is counted.
    pub score: F32,
    /// The room's light, 0 to 1. A new room has 0.
    pub light: F32,
    /// Whether the room is a pool: its first listed tile is on the ground
    /// level and its pool cell is below 16.
    pub pool: bool,
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

    /// Marks a room as not counted, so a later [`World::update_rooms`]
    /// counts it again, and notes that its level has a room waiting
    /// (0x466b07, 0x466b55; signals 0xf3 and 0xf1, and the Refresh
    /// primitive). The room is named by an object's room word; a word that
    /// names no room does nothing.
    pub fn mark_room(&mut self, room: i16) {
        if let Some(found) = self.rooms.rooms.get_mut(&room) {
            found.dirty = true;
            match (room as u16 >> 10) & 3 {
                0 => self.rooms.pending |= 1,
                1 => self.rooms.pending |= 2,
                _ => {}
            }
        }
    }

    /// Marks every room as not counted and both levels as waiting
    /// (0x466c01, 0x466ba3). The original does this when the lighting
    /// changes (signal 0xdd), which a load sends once the house is read,
    /// and when the game goes to live mode (0x416534).
    pub fn mark_all_rooms(&mut self) {
        for room in self.rooms.rooms.values_mut() {
            room.dirty = true;
        }
        self.rooms.pending = 3;
    }

    /// The room update: counts every room that is marked as not counted,
    /// works out its light and its score, then the lot's outdoor score and
    /// indoor area (0x466c3c). Returns the IDs of the rooms counted, in
    /// the order they were counted.
    ///
    /// `floor` is 0 for every room, or the level (1 or 2) whose rooms are
    /// to be counted. `standing` lists the objects standing on a tile, in
    /// the order the lot links them, everything they hold included
    /// (0x43f331); `sims-vm` supplies it, because it holds the object
    /// definitions.
    ///
    /// For each marked room, in ID order:
    ///
    /// 1. The light the room shows is noted ([`Room::shown_light`]).
    /// 2. Its counts are zeroed and its mark cleared (0x4678b7).
    /// 3. Whether it is a pool is worked out from its first listed tile.
    /// 4. Each listed tile is counted, repeats included (0x467bb1), and
    ///    then each object standing on it (0x467e05).
    /// 5. Its light and score are worked out ([`Tuning::score`],
    ///    0x467980).
    /// 6. If the light it shows now differs from the one noted, the room
    ///    is announced as changed (signal 0xf1), and the lot answers by
    ///    marking it as not counted again (0x466b55). So a room whose
    ///    light changed leaves this update still marked.
    ///
    /// Then, over all rooms: global 27 takes the sum, over the indoor
    /// rooms, of the half tiles divided by 2, rounding each room down; the
    /// scores of the outdoor rooms are added up, and the lot's outdoor
    /// score is that sum divided by a constant, plus a bonus for the time
    /// of day (global 4), kept within -100 and 100. Last, the levels that
    /// were counted stop waiting ([`Rooms::pending`]), whatever step 6
    /// marked.
    ///
    /// A tile (0x467bb1): an outdoor room counts 2 half tiles and nothing
    /// else. An indoor room counts 2 half tiles for a tile with no
    /// diagonal wall, both covered if the tile has a floor, and each of
    /// the tile's four walls whose style can take a covering, with whether
    /// it has one. A tile with a diagonal wall counts 1 half tile, covered
    /// if the wall cell's first style or first pattern word is nonzero
    /// (they hold the two half floors), and the diagonal wall itself. A
    /// tile whose pool cell (array 9) is below 16 adds one to the room's
    /// count of good things.
    ///
    /// An object (0x467e05) is skipped if its data word 34 is not 0 or its
    /// room word (29) is negative. Otherwise it is counted; by its data
    /// word 59 it may count as one of two categories; a person
    /// (definition type 2) counts as a person. If it is a light source
    /// (flag 0x400 of data word 40) and stands on the lot, it counts as a
    /// light unless it is a window (type 8), and its data word 51 is
    /// added to the room's window light or lamp light. Its data word 39 is
    /// added up. Its data word 7 is added to the room's impact, outdoors
    /// only if it is negative, and an object whose word 7 is not negative
    /// counts as a good thing.
    ///
    /// Ported from the old prototype
    /// (legacy/crates/sims-sim/src/lot_environment.rs and
    /// lot_environment/manager.rs, objects.rs), and read again from the
    /// original's code. For the Goth house every count, the light and the
    /// score of all 11 rooms, the lot's outdoor sum and score, global 27,
    /// the marks and the waiting levels were compared with a trace of the
    /// original at two moments and were equal; see `docs/decisions.md`.
    ///
    /// Not modeled, being presentation: the lights the update places in a
    /// room, the lighting of the tiles, and the redrawing. Not ported:
    /// whether an indoor room is lit because someone is in it (0x468802).
    /// A room where that would matter fails with
    /// [`RoomError::Occupancy`].
    pub fn update_rooms(
        &mut self,
        tuning: &Tuning,
        floor: u8,
        standing: &mut ListStanding<'_>,
    ) -> Result<Vec<i16>, RoomError> {
        if self.globals.len() != 64 {
            return Err(RoomError::NoContext);
        }
        if floor > 2 {
            return Err(RoomError::Level(i32::from(floor)));
        }
        let period = self.globals[TIME_OF_DAY];
        let daylight = Tuning::daylight(period);
        let ids: Vec<i16> = self.rooms.rooms.keys().copied().collect();
        let mut counted = Vec::new();
        for id in ids {
            let index = usize::from((id as u16 >> 10) & 3);
            let room = &self.rooms.rooms[&id];
            if (floor != 0 && index + 1 != usize::from(floor)) || !room.dirty {
                continue;
            }
            if index > 1 {
                return Err(RoomError::Level(index as i32 + 1));
            }
            let before = room.shown_light(id)?;
            let outdoors = room.outdoors;
            let tiles = room.tiles.clone();
            let mut count = Room {
                outdoors,
                ..Room::default()
            };
            // The pool array is optional in a house file. A lot without
            // one has no pool.
            let has_pool = self.arrays.contains_key(&POOL);
            if let Some(first) = tiles.first().filter(|_| has_pool) {
                let pool = array(&self.arrays, POOL, 1)?;
                count.pool = first[2] == 1 && pool[at(first[0].into(), first[1].into())] < 16;
            }
            for tile in &tiles {
                {
                    let level = Level {
                        level: index as u8 + 1,
                        walls: array(&self.arrays, WALLS[index], WALL_CELL)?,
                        floors: array(&self.arrays, FLOORS[index], 2)?,
                    };
                    let pool = match has_pool {
                        true => Some(array(&self.arrays, POOL, 1)?),
                        false => None,
                    };
                    count.count_tile(&level, pool, *tile)?;
                }
                for object in standing(self, *tile)? {
                    let data = &self
                        .objects
                        .get(&object.id)
                        .ok_or(RoomError::Standing(object.id))?
                        .data;
                    count.count_object(data, &object)?;
                }
            }
            let (light, score) = tuning.score(&count, daylight);
            count.light = light;
            count.score = score;
            let after = count.shown_light(id)?;
            let room = self.rooms.rooms.get_mut(&id).ok_or(RoomError::NoRooms)?;
            count.tiles = tiles;
            *room = count;
            counted.push(id);
            // The original compares the two with FCOMP and skips the
            // signal when they are equal or unordered.
            if matches!(
                before.ext().partial_cmp(&after.ext()),
                Some(Ordering::Less | Ordering::Greater)
            ) {
                self.mark_room(id);
            }
        }
        let mut indoor = 0i32;
        let mut sum = F32::ZERO;
        for room in self.rooms.rooms.values() {
            if room.outdoors {
                sum = add(room.score.ext(), sum.ext()).to_f32();
            } else {
                indoor = indoor.wrapping_add(room.half_tiles / 2);
            }
        }
        self.rooms.outdoor_sum = sum;
        self.rooms.outdoor_score = tuning.outdoor_score(sum, period);
        self.globals[INDOOR_AREA] = indoor as i16;
        match floor {
            0 => self.rooms.pending = 0,
            1 => self.rooms.pending &= !1,
            _ => self.rooms.pending &= !2,
        }
        Ok(counted)
    }

    /// Runs the room update if any level has a room waiting (0x467094):
    /// for one level if only that level waits, otherwise for every room.
    /// The original does this before it draws a frame, and at the end of
    /// reading a house. Returns the rooms counted.
    pub fn update_pending_rooms(
        &mut self,
        tuning: &Tuning,
        standing: &mut ListStanding<'_>,
    ) -> Result<Vec<i16>, RoomError> {
        match self.rooms.pending {
            0 => Ok(Vec::new()),
            1 => self.update_rooms(tuning, 1, standing),
            2 => self.update_rooms(tuning, 2, standing),
            3 => self.update_rooms(tuning, 0, standing),
            // The original does nothing for any other value. Nothing here
            // sets one.
            _ => Ok(Vec::new()),
        }
    }
}

/// What lists the objects standing on a tile for the room update: given
/// the lot and a tile as column, row and level, the objects on it in the
/// order the lot links them. See [`World::update_rooms`].
pub type ListStanding<'a> = dyn FnMut(&mut World, [u8; 3]) -> Result<Vec<Standing>, RoomError> + 'a;

/// One object standing on a tile, as the room update is told of it. See
/// [`World::update_rooms`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub id: u16,
    /// The object's type: word 9 of its definition. 2 is a person and 8 a
    /// window.
    pub kind: i16,
    /// Whether the object stands on the lot (0x432595). Only asked of a
    /// light source, an object with flag 0x400 in data word 40; for any
    /// other object the room update does not look at it.
    pub on_lot: bool,
}

impl Room {
    /// The light the room shows (0x4688bf, and inline in 0x466c3c): 0.8
    /// for a pool, and for a dark room with no lights that knows someone
    /// is in it; otherwise the room's light.
    ///
    /// A person tells its room as it enters and leaves (0x468802). An
    /// outdoor room and a pool ignore that, so they never know. For an
    /// indoor room it is not ported, and this fails for the one case
    /// where the answer depends on it: an indoor room that is no pool,
    /// has no lights, has people standing in it, and whose light is below
    /// 0.8.
    pub fn shown_light(&self, id: i16) -> Result<F32, RoomError> {
        let lit = F32::from_bits(LIT);
        if self.pool {
            return Ok(lit);
        }
        if !self.outdoors
            && self.lights == 0
            && self.people > 0
            && !matches!(
                self.light.ext().partial_cmp(&lit.ext()),
                Some(Ordering::Equal | Ordering::Greater)
            )
        {
            return Err(RoomError::Occupancy(id));
        }
        Ok(self.light)
    }

    /// Counts one listed tile (0x467bb1).
    fn count_tile(
        &mut self,
        level: &Level<'_>,
        pool: Option<&[u8]>,
        tile: [u8; 3],
    ) -> Result<(), RoomError> {
        let (x, y) = (usize::from(tile[0]), usize::from(tile[1]));
        let cell = at(x, y);
        if pool.is_some_and(|pool| pool[cell] < 16) {
            self.good = self.good.wrapping_add(1);
        }
        let segments = level.segments(cell);
        if self.outdoors {
            self.half_tiles = self.half_tiles.wrapping_add(2);
        } else if segments & wall::DIAGONAL == 0 {
            self.half_tiles = self.half_tiles.wrapping_add(2);
            if level.floor(cell) != 0 {
                self.covered_half_tiles = self.covered_half_tiles.wrapping_add(2);
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
                    self.walls = self.walls.wrapping_add(1);
                    if level.pattern(cell, bit) != 0 {
                        self.covered_walls = self.covered_walls.wrapping_add(1);
                    }
                }
            }
        } else {
            self.half_tiles = self.half_tiles.wrapping_add(1);
            // A diagonal wall's style is the cell's second style word,
            // and its two sides are the last two pattern words.
            if decorated(level.word(cell, 1)) {
                self.walls = self.walls.wrapping_add(1);
                if level.word(cell, 4) != 0 || level.word(cell, 5) != 0 {
                    self.covered_walls = self.covered_walls.wrapping_add(1);
                }
            }
            if level.word(cell, 0) != 0 || level.word(cell, 2) != 0 {
                self.covered_half_tiles = self.covered_half_tiles.wrapping_add(1);
            }
        }
        Ok(())
    }

    /// Counts one object standing on a listed tile (0x467e05).
    fn count_object(&mut self, data: &[i16], object: &Standing) -> Result<(), RoomError> {
        let word = |index: usize| {
            data.get(index)
                .copied()
                .ok_or(RoomError::Standing(object.id))
        };
        if word(34)? != 0 || word(29)? < 0 {
            return Ok(());
        }
        self.objects = self.objects.wrapping_add(1);
        match word(59)? {
            4 => self.category_4 = self.category_4.wrapping_add(1),
            5..=7 => self.category_5_to_7 = self.category_5_to_7.wrapping_add(1),
            _ => {}
        }
        if object.kind == 2 {
            self.people = self.people.wrapping_add(1);
        }
        if word(40)? as u16 & 0x400 != 0 && object.on_lot {
            let window = object.kind == 8;
            if !window {
                self.lights = self.lights.wrapping_add(1);
            }
            let light = i32::from(word(51)?);
            if window {
                self.window_light = self.window_light.wrapping_add(light);
            } else {
                self.lamp_light = self.lamp_light.wrapping_add(light);
            }
        }
        self.word_39 = self.word_39.wrapping_add(i32::from(word(39)?));
        let impact = word(7)?;
        if !self.outdoors || impact < 0 {
            self.impact = self.impact.wrapping_add(i32::from(impact));
        }
        if impact >= 0 {
            self.good = self.good.wrapping_add(1);
        }
        Ok(())
    }
}

/// The constants the room scores are worked out with (0x4665d1). The
/// original reads them by name from the tuning table 3 of the global
/// file, with these defaults for a name the table lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    pub object_light: F32,
    pub area_clip: F32,
    pub area_zero: F32,
    pub area_slope: F32,
    pub light_zero: F32,
    pub light_slope: F32,
    pub impact_area_min: F32,
    pub impact_area_max: F32,
    pub impact_multiplier: F32,
    pub outdoor_impact: F32,
    pub outdoor_good: F32,
    pub outdoor_divisor: F32,
    pub daylight_bonus: F32,
    pub dawn_dusk_bonus: F32,
    pub nighttime_bonus: F32,
    pub floor_zero: F32,
    pub floor_slope: F32,
    pub wall_zero: F32,
    pub wall_slope: F32,
}

impl Default for Tuning {
    /// The original's defaults.
    fn default() -> Self {
        Self::from_lookup(|_| None)
    }
}

/// 0.01, 0.5, 0.8 and 1.0 as floats, and 0.99 as a double: constants of
/// the original's code.
const HUNDREDTH: u32 = 0x3c23_d70a;
const HALF: u32 = 0x3f00_0000;
const LIT: u32 = 0x3f4c_cccd;
const ONE: u32 = 0x3f80_0000;
const NEARLY_ONE: u64 = 0x3fef_ae14_7ae1_47ae;

/// The original subtracts two floats it has just read, in the FPU, and
/// stores the difference as a float.
fn slope(full: f32, zero: f32) -> F32 {
    (F32::from_f32(full).ext() - F32::from_f32(zero).ext()).to_f32()
}

fn above(a: Ext, b: Ext) -> bool {
    a.partial_cmp(&b) == Some(Ordering::Greater)
}

fn below_or_unordered(a: Ext, b: Ext) -> bool {
    !matches!(a.partial_cmp(&b), Some(Ordering::Equal | Ordering::Greater))
}

impl Tuning {
    /// Reads the constants by name. `lookup` gives the table's value for
    /// a name, or None for a name the table lacks.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<f32>) -> Self {
        let get = |name: &str, fallback: f32| lookup(name).unwrap_or(fallback);
        let area_zero = get("area score for zero tiles", -30.0);
        let light_zero = get("light score for 0.0", -20.0);
        let floor_zero = get("floor score for 0%", -40.0);
        let wall_zero = get("wall score for 0%", -40.0);
        let f = F32::from_f32;
        Self {
            object_light: f(get("object light contribution factor", 30.0)),
            area_clip: f(get("area clip to score", 60.0)),
            area_zero: f(area_zero),
            area_slope: slope(get("area score for clip tile", 30.0), area_zero),
            light_zero: f(light_zero),
            light_slope: slope(get("light score for 1.0", 20.0), light_zero),
            impact_area_min: f(get("room impact effective area min", 10.0)),
            impact_area_max: f(get("room impact effective area max", 45.0)),
            impact_multiplier: f(get("room impact multiplier", 10.0)),
            outdoor_impact: f(get("outdoor room impact factor", 2.0)),
            outdoor_good: f(get("outdoor good object count factor", 1.0)),
            outdoor_divisor: f(get("outdoor object divisor", 3.0)),
            daylight_bonus: f(get("outdoor daylight bonus", 0.0)),
            dawn_dusk_bonus: f(get("outdoor dawn dusk bonus", -20.0)),
            nighttime_bonus: f(get("outdoor nighttime bonus", -50.0)),
            floor_zero: f(floor_zero),
            floor_slope: slope(get("floor score for 100%", 0.0), floor_zero),
            wall_zero: f(wall_zero),
            wall_slope: slope(get("wall score for 100%", 0.0), wall_zero),
        }
    }

    /// How much daylight there is, by the time of day in global 4
    /// (0x46740f): all of it by day (0), half at dawn and dusk (1 and 3),
    /// none at night (2). The value stays in an FPU register.
    ///
    /// The original multiplies this by a setting that is 1.0 in a trace
    /// of the running game. What changes that setting is not known, so it
    /// is taken to be 1.0: **unverified**.
    pub fn daylight(period: i16) -> Ext {
        match period {
            1 | 3 => F32::from_bits(HALF).ext(),
            2 => Ext::ZERO,
            _ => F32::from_bits(ONE).ext(),
        }
    }

    /// A counted room's light and score (0x467980), in the original's
    /// order of operations: every value it stores as a float on the way
    /// is stored here too.
    ///
    /// The original's FPU runs this with its precision control set to
    /// single precision, so every operation's result is rounded to 24
    /// bits ([`Ext::single`]). A trace of the running game shows it: one
    /// room's score there is the sum rounded after each addition, three
    /// units in the last place away from the sum rounded once.
    ///
    /// The light of an outdoor room is the daylight. Indoors it is the
    /// lamp light plus the window light, each as hundredths per tile of
    /// area times a constant, the window light also times the daylight
    /// and no more than the daylight. Below 0.01 it is 0, and above 0.99
    /// it is 1.
    ///
    /// The score of an outdoor room is its good things and its impact,
    /// each times a constant. Indoors it is the sum of five parts: the
    /// share of the walls that are covered, the share of the floor that
    /// is, the impact over the area (the area kept between two bounds),
    /// the light, and the area up to a limit; all kept within -100 and
    /// 100.
    pub fn score(&self, room: &Room, daylight: Ext) -> (F32, F32) {
        let int = Ext::from_i32;
        let area = room.half_tiles / 2;
        let hundredth = F32::from_bits(HUNDREDTH).ext();
        let light = if room.outdoors {
            daylight.to_f32()
        } else {
            let per_tile = |light: i32| div(mul(int(light), hundredth), int(area));
            let lamp = mul(per_tile(room.lamp_light), self.object_light.ext()).to_f32();
            let window = mul(
                mul(per_tile(room.window_light), daylight),
                self.object_light.ext(),
            );
            // The original stores the window light, then replaces the
            // stored value with the daylight if the register is above it.
            let window = if above(window, daylight) {
                daylight.to_f32()
            } else {
                window.to_f32()
            };
            let value = add(window.ext(), lamp.ext());
            if below_or_unordered(value, hundredth) {
                F32::ZERO
            } else if above(value, Ext::from_f64_bits(NEARLY_ONE)) {
                F32::from_bits(ONE)
            } else {
                value.to_f32()
            }
        };
        let score = if room.outdoors {
            add(
                mul(int(room.good), self.outdoor_good.ext()),
                mul(int(room.impact), self.outdoor_impact.ext()),
            )
            .to_f32()
        } else {
            let area = int(area);
            let clip = self.area_clip.ext();
            let clipped = if above(area, clip) { clip } else { area };
            let area_score = add(
                mul(div(clipped, clip), self.area_slope.ext()),
                self.area_zero.ext(),
            )
            .to_f32();
            let light_score = add(
                mul(light.ext(), self.light_slope.ext()),
                self.light_zero.ext(),
            )
            .to_f32();
            let (least, most) = (self.impact_area_min.ext(), self.impact_area_max.ext());
            let effective = if below_or_unordered(area, least) {
                least
            } else if above(area, most) {
                most
            } else {
                area
            };
            let walls = add(
                mul(
                    div(int(room.covered_walls), int(room.walls)),
                    self.wall_slope.ext(),
                ),
                self.wall_zero.ext(),
            );
            let floors = add(
                mul(
                    div(int(room.covered_half_tiles), int(room.half_tiles)),
                    self.floor_slope.ext(),
                ),
                self.floor_zero.ext(),
            );
            let impact = div(
                mul(int(room.impact), self.impact_multiplier.ext()),
                effective,
            );
            let value = add(
                add(add(add(walls, floors), impact), light_score.ext()),
                area_score.ext(),
            );
            if below_or_unordered(value, int(-100)) {
                int(-100).to_f32()
            } else if above(value, int(100)) {
                int(100).to_f32()
            } else {
                value.to_f32()
            }
        };
        (light, score)
    }

    /// The lot's outdoor score from the sum of the outdoor rooms' scores
    /// (the end of 0x466c3c). The sum is divided by a constant and stored;
    /// for a time of day 0 to 3 the bonus for it is added to the value
    /// still in the register and the result stored over it. Then it is
    /// kept within -100 and 100.
    pub fn outdoor_score(&self, sum: F32, period: i16) -> F32 {
        let share = div(sum.ext(), self.outdoor_divisor.ext());
        let value = match period {
            0 => add(share, self.daylight_bonus.ext()).to_f32(),
            1 | 3 => add(share, self.dawn_dusk_bonus.ext()).to_f32(),
            2 => add(share, self.nighttime_bonus.ext()).to_f32(),
            _ => share.to_f32(),
        };
        let (least, most) = (Ext::from_i32(-100), Ext::from_i32(100));
        if above(least, value.ext()) {
            least.to_f32()
        } else if above(value.ext(), most) {
            most.to_f32()
        } else {
            value
        }
    }
}

/// The FPU's operations as the room code runs them: with single
/// precision. See [`Tuning::score`].
fn add(a: Ext, b: Ext) -> Ext {
    (a + b).single()
}

fn mul(a: Ext, b: Ext) -> Ext {
    (a * b).single()
}

fn div(a: Ext, b: Ext) -> Ext {
    (a / b).single()
}
