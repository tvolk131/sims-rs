//! The lot's terrain, floors and walls, read from the world's arrays.
//!
//! A [`sims_sim::World`] holds them either as the house file saves them
//! (`World::from_house`: a floor is one byte, a wall cell 8 bytes) or as
//! a running lot holds them (after `World::read_arrays`: a floor is a
//! 16-bit word, a wall cell 14 bytes with 16-bit styles and patterns). This
//! module reads both and never writes the world.

use crate::{Error, Result};
use sims_sim::World;
use std::collections::BTreeMap;

/// The side of the lot's arrays.
pub const FULL: usize = 64;

/// A floor ID that means a diagonal wall splits the tile; its two halves'
/// floors are in the wall cell (see [`WallCell::split_floors`]).
pub const SPLIT_FLOOR: u16 = 255;

/// Wall segment bits of a cell. A cell owns the walls along its low-x edge
/// ([`WEST`]) and low-y edge ([`NORTH`]); [`EAST`] and [`SOUTH`] repeat the
/// neighbours' walls. [`DESCENDING`] runs from corner (1, 0) to (0, 1),
/// [`ASCENDING`] from (0, 0) to (1, 1).
pub const WEST: u8 = 1;
pub const NORTH: u8 = 2;
pub const EAST: u8 = 4;
pub const SOUTH: u8 = 8;
pub const DESCENDING: u8 = 0x10;
pub const ASCENDING: u8 = 0x20;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WallCell {
    pub segments: u8,
    pub flags: u8,
    /// The styles (shapes) of the [`WEST`] and [`NORTH`] walls.
    pub styles: [u16; 2],
    /// The wallpapers: `[0]` the west wall's side in this cell, `[1]` the
    /// north wall's side in this cell, `[2]` this cell's side of the
    /// neighbouring south wall, `[3]` this cell's side of the east wall.
    /// A diagonal wall's two sides are `[3]` (the side its normal points
    /// to) and `[2]`.
    pub patterns: [u16; 4],
}

impl WallCell {
    /// The floors of the two halves of a split tile: the half holding
    /// corner (0, 0) for a descending wall, or corner (0, 1) for an
    /// ascending one, then the other half. Which half takes which is
    /// unverified.
    pub fn split_floors(&self) -> [u16; 2] {
        [self.styles[0], self.patterns[0]]
    }
}

#[derive(Debug, Clone)]
pub struct Level {
    pub floors: Vec<u16>,
    pub walls: Vec<WallCell>,
    /// The room of each tile, or a negative value for none, if the world's
    /// rooms are worked out.
    pub rooms: Option<Vec<i16>>,
}

#[derive(Debug, Clone)]
pub struct Lot {
    /// The active size: tiles 0 to `size - 1` in each direction.
    pub size: usize,
    /// Each tile's four corner heights, in terrain steps.
    pub heights: Vec<[u8; 4]>,
    /// Each tile's grass value (Arry 6).
    pub grass: Vec<i8>,
    pub levels: [Level; 2],
    /// The rooms that are outdoors, by ID.
    pub outdoor_rooms: std::collections::BTreeSet<i16>,
    /// The light byte of each room, by ID: the light the room shows
    /// ([`sims_sim::rooms::Room::shown_light`]) times 255, as the original
    /// stores it for the room's tiles (0x46772d).
    pub room_lights: BTreeMap<i16, u8>,
    /// The light each room shows, by ID ([`room_fraction`]), as the
    /// original passes it to the drawing context for a wall face or the
    /// cap at a wall's exposed end (0x4688bf, 0x53dc78, 0x53c9d3).
    pub room_fractions: BTreeMap<i16, f32>,
    /// The light room 0, the outdoors, shows ([`room_fraction`]), as the
    /// original reads it for the wall-top trim (0x53dbcd); None without a
    /// room 0.
    pub outdoor_light: Option<f32>,
    /// The time of day (global 4: 0 day, 1 dusk, 2 night, 3 dawn). It
    /// sets the brightest a lit tile is drawn ([`Lot::brightest`]) and
    /// where the sun stands over the grass ([`crate::grass`]).
    pub time_of_day: i16,
    /// The two triangles of each tile a diagonal wall splits, by level (1
    /// or 2), column and row: which triangle (by the corner it holds: 1
    /// south-west, 2 south-east, 3 north-east, 4 north-west) and its room.
    pub halves: BTreeMap<[u8; 3], [(u8, i16); 2]>,
    /// Each tile's lamps, by level (0 or 1) and cell: see [`Lamps`].
    pub lamps: [Vec<Lamps>; 2],
    /// The camera rotation (`0..=3`) that `levels[0].floors`'s road-dash
    /// tiles (`10` and `11`) are oriented for: `HOUS 0`'s saved
    /// `camera_direction & 3` (`0` by default). The original swaps `10` and
    /// `11` in place across level 1's floor array on each odd-parity view
    /// turn (`0x4a7dfe`, `0x4a8042` to `0x4a80b1`) and loads without
    /// swapping (`0x4a861f`), so saved arrays keep their last turn's
    /// orientation.
    pub floor_rotation: u8,
}

/// The offsets from a tile to the two nearest lit lamps of its room, in
/// tiles, or none (the original's 4-byte record per tile, 0x7f for none,
/// which 0x4b4e8b fills).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lamps(pub [Option<[i8; 2]>; 2]);

/// How many tiles a lamp marks around it, each way: the light radius of 12
/// over 3, rounded (0x4b4e8b).
const LAMP_REACH: i32 = 4;

impl Lamps {
    /// Adds a lamp at `offset` (0x4b5037): into the first free place, or,
    /// if both are taken, over the first if it is nearer than that, else
    /// over the second if it is nearer than that.
    fn add(&mut self, offset: [i8; 2]) {
        let distance = |o: [i8; 2]| i32::from(o[0]).pow(2) + i32::from(o[1]).pow(2);
        let new = distance(offset);
        match self.0 {
            [None, _] => self.0[0] = Some(offset),
            [Some(_), None] => self.0[1] = Some(offset),
            [Some(a), Some(b)] => {
                if new < distance(a) {
                    self.0[0] = Some(offset);
                } else if new < distance(b) {
                    self.0[1] = Some(offset);
                }
            }
        }
    }
}

impl Lot {
    pub fn from_world(world: &World) -> Result<Self> {
        let size = usize::try_from(world.size)
            .ok()
            .filter(|s| (1..=FULL).contains(s))
            .ok_or_else(|| Error::new(format!("lot size {} is out of range", world.size)))?;
        let array = |id: u16| {
            world
                .arrays
                .get(&id)
                .filter(|a| {
                    a.width == FULL
                        && a.height == FULL
                        && Some(a.data.len()) == (FULL * FULL).checked_mul(a.stride)
                })
                .ok_or_else(|| Error::new(format!("the lot has no 64 by 64 array {id}")))
        };
        let heights = {
            let a = array(0)?;
            check(a.stride == 4, 0)?;
            a.data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| [c[0], c[1], c[2], c[3]])
                .collect()
        };
        let grass = match world.arrays.get(&6) {
            Some(a) if a.stride == 1 && a.data.len() == FULL * FULL => {
                a.data.iter().map(|&b| b as i8).collect()
            }
            _ => vec![0; FULL * FULL],
        };
        let level = |index: usize| -> Result<Level> {
            let floors = array([1, 101][index])?;
            let floors = match floors.stride {
                1 => floors.data.iter().map(|&b| u16::from(b)).collect(),
                2 => floors
                    .data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect(),
                _ => return Err(Error::new("floor array stride is neither 1 nor 2")),
            };
            let walls = array([2, 102][index])?;
            let walls = match walls.stride {
                8 => walls
                    .data
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| WallCell {
                        segments: c[0],
                        flags: c[1],
                        styles: [c[2], c[3]].map(u16::from),
                        patterns: [c[4], c[5], c[6], c[7]].map(u16::from),
                    })
                    .collect(),
                14 => walls
                    .data
                    .as_chunks::<14>()
                    .0
                    .iter()
                    .map(|c| {
                        let w = |n: usize| u16::from_le_bytes([c[2 + n * 2], c[3 + n * 2]]);
                        WallCell {
                            segments: c[0],
                            flags: c[1],
                            styles: [w(0), w(1)],
                            patterns: [w(2), w(3), w(4), w(5)],
                        }
                    })
                    .collect(),
                _ => return Err(Error::new("wall array stride is neither 8 nor 14")),
            };
            let rooms = Some(world.rooms.map[index].clone()).filter(|m| m.len() == FULL * FULL);
            Ok(Level {
                floors,
                walls,
                rooms,
            })
        };
        let outdoor_rooms = world
            .rooms
            .rooms
            .iter()
            .filter(|(_, r)| r.outdoors)
            .map(|(&id, _)| id)
            .collect();
        let room_lights = world
            .rooms
            .rooms
            .iter()
            .map(|(&id, r)| (id, room_light(r)))
            .collect();
        let room_fractions = world
            .rooms
            .rooms
            .iter()
            .map(|(&id, r)| (id, room_fraction(r)))
            .collect();
        let outdoor_light = world.rooms.rooms.get(&0).map(room_fraction);
        let halves = world
            .rooms
            .halves
            .iter()
            .map(|(&tile, h)| (tile, h.map(|h| (h.triangle, h.room))))
            .collect();
        let levels = [level(0)?, level(1)?];
        let lamps = [0, 1].map(|index| lamps(world, size, &levels[index], &halves, index));
        Ok(Self {
            size,
            heights,
            grass,
            levels,
            outdoor_rooms,
            room_lights,
            room_fractions,
            outdoor_light,
            time_of_day: world.globals.get(4).copied().unwrap_or(0),
            halves,
            lamps,
            floor_rotation: 0,
        })
    }

    /// Sets [`Lot::floor_rotation`] to `rotation & 3`, matching a house
    /// file's saved `camera_direction` (`HOUS 0`).
    pub fn with_floor_rotation(mut self, rotation: u8) -> Self {
        self.floor_rotation = rotation & 3;
        self
    }

    /// The middle of what is built on the ground level (the box around its
    /// walls), or of the lot if nothing is, in sixteenths of a tile.
    pub fn built_center(&self) -> [i32; 2] {
        let mut low = [i32::MAX; 2];
        let mut high = [i32::MIN; 2];
        for x in 0..self.size {
            for y in 0..self.size {
                if self.levels[0].walls[Self::cell(x, y)].segments != 0 {
                    for (i, n) in [x as i32, y as i32].into_iter().enumerate() {
                        low[i] = low[i].min(n);
                        high[i] = high[i].max(n + 1);
                    }
                }
            }
        }
        if low[0] > high[0] {
            let half = self.size as i32 * 8;
            return [half, half];
        }
        [(low[0] + high[0]) * 8, (low[1] + high[1]) * 8]
    }

    pub fn cell(x: usize, y: usize) -> usize {
        x * FULL + y
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.size && (y as usize) < self.size
    }
}

impl Lot {
    /// The room at fine position `fine` (sixteenths of a tile) on a level
    /// (0 or 1): the tile's room, or on a tile a diagonal wall splits, the
    /// room of the triangle the point is in. None off the lot, outside
    /// every room, or if the rooms are not worked out.
    pub fn room_at(&self, level: usize, fine: [i32; 2]) -> Option<i16> {
        let [x, y] = fine.map(|f| f.div_euclid(16));
        if !self.contains(x, y) {
            return None;
        }
        let rooms = self.levels.get(level)?.rooms.as_ref()?;
        let room = rooms[Self::cell(x as usize, y as usize)];
        if room == sims_sim::rooms::NO_ROOM {
            let key = [level as u8 + 1, x as u8, y as u8];
            let halves = self.halves.get(&key)?;
            let [fx, fy] = fine.map(|f| f.rem_euclid(16));
            // The corner each triangle holds: 1 is (0, 1), 2 is (1, 1),
            // 3 is (1, 0) and 4 is (0, 0), with x east and y south.
            let inside = |triangle: u8| match triangle {
                1 => fy > fx,
                2 => fx + fy > 16,
                3 => fx > fy,
                _ => fx + fy < 16,
            };
            let half = halves
                .iter()
                .find(|(t, _)| inside(*t))
                .unwrap_or(&halves[0]);
            return (half.1 >= 0).then_some(half.1);
        }
        (room >= 0).then_some(room)
    }

    /// The brightest a lit tile is drawn at this time of day
    /// ([`crate::light::brightest`]).
    pub fn brightest(&self) -> f32 {
        crate::light::brightest(self.time_of_day)
    }

    /// The light byte at a fine position on a level (0 or 1): its room's
    /// light. Off the lot and outside every room it is full light.
    pub fn light_at(&self, level: usize, fine: [i32; 2]) -> u8 {
        self.room_light_at(level, fine).unwrap_or(255)
    }

    /// The light byte of the room at a fine position on a level (0 or 1),
    /// or None off the lot, outside every room, or for a room without a
    /// light.
    pub fn room_light_at(&self, level: usize, fine: [i32; 2]) -> Option<u8> {
        self.room_at(level, fine)
            .and_then(|room| self.room_lights.get(&room).copied())
    }

    /// The light the room at a fine position on a level (0 or 1) shows,
    /// as a fraction ([`room_fraction`]), or None off the lot or outside
    /// every room.
    pub fn room_fraction_at(&self, level: usize, fine: [i32; 2]) -> Option<f32> {
        self.room_at(level, fine)
            .and_then(|room| self.room_fractions.get(&room).copied())
    }
}

/// The lamps of every tile of a level (`index` 0 or 1).
///
/// The original keeps the records as it counts the rooms (0x467e05): an
/// object counted there that is a light source (data word 40 flag 0x400),
/// gives light (word 51 not 0) and has word 16 set to 1 first takes its
/// marks off the tiles around it (0x4b50b5), then marks them again
/// (0x4b4e8b). Every tile within four tiles each way (a square), inside the
/// lot's inner tiles and of the lamp tile's room (for a split tile, either
/// half's), takes the offset from the lamp to it. An object counted with
/// no light only takes its marks off.
///
/// Here the records are worked out afresh from the objects as they stand,
/// lamps in object order. The original's records instead build up from
/// the room counts, rooms in its own order; where three or more lamps
/// reach a tile, or a lamp went dark, the two could differ. That has not
/// been compared.
fn lamps(
    world: &World,
    size: usize,
    level: &Level,
    halves: &BTreeMap<[u8; 3], [(u8, i16); 2]>,
    index: usize,
) -> Vec<Lamps> {
    let mut records = vec![Lamps::default(); FULL * FULL];
    let Some(rooms) = &level.rooms else {
        return records;
    };
    let inner = |n: i32| n >= 1 && n < size as i32 - 1;
    for object in world.objects.values() {
        let [x, y, on] = object.position;
        if !gives_light(object) || on != index as i32 + 1 {
            continue;
        }
        let [x, y] = [x, y].map(|f| f.div_euclid(16));
        if !inner(x) || !inner(y) {
            continue;
        }
        let room = rooms[Lot::cell(x as usize, y as usize)];
        for dy in -LAMP_REACH..=LAMP_REACH {
            for dx in -LAMP_REACH..=LAMP_REACH {
                let [qx, qy] = [x + dx, y + dy];
                if !inner(qx) || !inner(qy) {
                    continue;
                }
                let cell = Lot::cell(qx as usize, qy as usize);
                let same = rooms[cell] == room
                    || (rooms[cell] == sims_sim::rooms::NO_ROOM
                        && halves
                            .get(&[index as u8 + 1, qx as u8, qy as u8])
                            .is_some_and(|h| h.iter().any(|&(_, r)| r == room)));
                if same {
                    records[cell].add([dx as i8, dy as i8]);
                }
            }
        }
    }
    records
}

/// Whether an object is a lamp that is lit, as the original's room count
/// takes one (0x467e05): not hidden (data word 34 is 0), in a room (word
/// 29 not negative), a light source (word 40 flag 0x400) that gives light
/// (word 51 not 0) and is on (word 16 is 1). Such an object marks the
/// tiles around it ([`Lot`]'s lamp records) and gives its room a light
/// that falls on the people there (`sims_people::lot::lamps`).
pub fn gives_light(object: &sims_sim::Object) -> bool {
    let word = |i: usize| object.data.get(i).copied().unwrap_or(0);
    word(34) == 0 && word(29) >= 0 && word(40) as u16 & 0x400 != 0 && word(51) != 0 && word(16) == 1
}

/// The light byte the original stores for a room's tiles: the light the
/// room shows ([`room_fraction`]) as [`light_byte`] makes it.
pub fn room_light(room: &sims_sim::rooms::Room) -> u8 {
    light_byte(room_fraction(room))
}

/// The light a room shows ([`sims_sim::rooms::Room::shown_light`]). A
/// room the room update has never counted has no light yet (every
/// counted room has at least one tile); the outdoors is drawn in daylight
/// then, so a house read without a load still shows, and indoors keeps
/// its light of 0.
pub fn room_fraction(room: &sims_sim::rooms::Room) -> f32 {
    if room.half_tiles == 0 && room.outdoors {
        1.0
    } else {
        room.shown_light().to_f32()
    }
}

/// A room's light as the byte the original stores for its tiles: the light
/// times 255, plus a half, truncated, keeping the low byte (0x46772d, the
/// old prototype's `light_byte` in
/// `legacy/crates/sims-sim/src/lot_environment/lighting.rs`). A light
/// that is not a number, or too large for a 64-bit integer, gives 0, the
/// low byte of the x87's "integer indefinite" result.
pub fn light_byte(light: f32) -> u8 {
    let value = (f64::from(light) * 255.0 + 0.5).trunc();
    // 2^63: the first value a 64-bit integer cannot hold.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if value.is_finite() && (-LIMIT..LIMIT).contains(&value) {
        (value as i64) as u8
    } else {
        0
    }
}

fn check(ok: bool, id: u16) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::new(format!(
            "array {id} has an unexpected cell size"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::lot::Array;

    fn array(stride: usize, len: usize) -> Array {
        Array {
            width: FULL,
            height: FULL,
            stride,
            encoding: 0,
            data: vec![0; len],
        }
    }

    #[test]
    fn a_tile_keeps_its_two_nearest_lamps_as_the_original_does() {
        let mut lamps = Lamps::default();
        lamps.add([3, 0]);
        lamps.add([0, 4]);
        assert_eq!(lamps, Lamps([Some([3, 0]), Some([0, 4])]));
        // Nearer than the first: it takes the first place, and the old
        // first is lost rather than moved down.
        lamps.add([1, 1]);
        assert_eq!(lamps, Lamps([Some([1, 1]), Some([0, 4])]));
        // Nearer than the second only.
        lamps.add([-2, 2]);
        assert_eq!(lamps, Lamps([Some([1, 1]), Some([-2, 2])]));
        // As far as the second: no change.
        lamps.add([2, -2]);
        assert_eq!(lamps, Lamps([Some([1, 1]), Some([-2, 2])]));
    }

    #[test]
    fn rooms_give_their_light_and_split_tiles_each_half_its_own() {
        use sims_sim::rooms::{Half, NO_ROOM, Room};
        let mut world = World::empty();
        let full = FULL * FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(id, array(stride, full * stride));
        }
        let mut map = vec![0i16; full];
        map[Lot::cell(1, 1)] = 1;
        map[Lot::cell(2, 2)] = NO_ROOM;
        world.rooms.map = [map, vec![0; full]];
        let room = |outdoors: bool, half_tiles: i32, light: f32| Room {
            outdoors,
            half_tiles,
            light: sims_core::F32::from_f32(light),
            ..Room::default()
        };
        world.rooms.rooms.insert(0, room(true, 2, 1.0));
        world.rooms.rooms.insert(1, room(false, 2, 0.25));
        world.rooms.rooms.insert(2, room(false, 2, 0.5));
        // A descending wall: triangle 4 holds corner (0, 0), 2 holds (1, 1).
        world.rooms.halves.insert(
            [1, 2, 2],
            [
                Half {
                    triangle: 4,
                    room: 1,
                },
                Half {
                    triangle: 2,
                    room: 2,
                },
            ],
        );
        let lot = Lot::from_world(&world).unwrap();
        assert_eq!(lot.light_at(0, [8, 8]), 255);
        assert_eq!(lot.light_at(0, [24, 24]), 64);
        assert_eq!(lot.light_at(0, [36, 36]), 64);
        assert_eq!(lot.light_at(0, [44, 44]), 128);
        assert_eq!(lot.room_at(0, [44, 44]), Some(2));
        // Off the lot is full light.
        assert_eq!(lot.light_at(0, [-1, 8]), 255);
        assert_eq!(lot.light_at(0, [8, 64 * 16]), 255);
        // An outdoor room never counted shows daylight; an indoor one its
        // light of 0.
        world.rooms.rooms.insert(0, room(true, 0, 0.0));
        world.rooms.rooms.insert(1, room(false, 0, 0.0));
        let lot = Lot::from_world(&world).unwrap();
        assert_eq!(lot.light_at(0, [8, 8]), 255);
        assert_eq!(lot.light_at(0, [24, 24]), 0);
    }

    /// A lit lamp marks the tiles within four of it in its own room and
    /// on the inner lot, each with its offset from the lamp; a dark one,
    /// or one on another level, marks nothing.
    #[test]
    fn a_lit_lamp_marks_the_tiles_of_its_room_around_it() {
        let mut world = World::empty();
        let full = FULL * FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(id, array(stride, full * stride));
        }
        // Room 1 is the tiles x < 6 (but the lot's edge); room 0 the rest.
        let mut map = vec![0i16; full];
        for y in 0..FULL {
            for x in 0..6 {
                map[Lot::cell(x, y)] = 1;
            }
        }
        world.rooms.map = [map, vec![0; full]];
        let lamp = |id: u16, tile: [i32; 2], level: i32, light: i16| {
            let mut data = vec![0i16; 64];
            (data[16], data[40], data[51]) = (1, 0x400, light);
            sims_sim::Object {
                id,
                guid: 0,
                footprint: [0; 4],
                position: [tile[0] * 16 + 8, tile[1] * 16 + 8, level],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: Vec::new(),
                data,
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: 0,
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
            }
        };
        world.objects.insert(1, lamp(1, [2, 10], 1, 1));
        world.objects.insert(2, lamp(2, [20, 20], 1, 0));
        world.objects.insert(3, lamp(3, [30, 30], 2, 1));
        let lot = Lot::from_world(&world).unwrap();
        let at = |level: usize, x, y| lot.lamps[level][Lot::cell(x, y)].0[0];
        assert_eq!(at(0, 2, 10), Some([0, 0]));
        assert_eq!(at(0, 5, 14), Some([3, 4]));
        assert_eq!(at(0, 1, 6), Some([-1, -4]));
        // Past four tiles, another room, the lot's edge row.
        assert_eq!(at(0, 2, 15), None);
        assert_eq!(at(0, 6, 10), None);
        assert_eq!(at(0, 0, 10), None);
        // The dark lamp marks nothing; the upstairs lamp only upstairs,
        // where this lot has no rooms.
        assert_eq!(at(0, 20, 20), None);
        assert_eq!(at(0, 30, 30), None);
        let marked = |level: usize| lot.lamps[level].iter().filter(|l| l.0[0].is_some()).count();
        // Columns 1 to 5 (the edge and room 0 left out), rows 6 to 14.
        assert_eq!(marked(0), 5 * 9);
    }

    #[test]
    fn light_bytes_round_half_up_and_keep_the_low_byte() {
        assert_eq!(light_byte(0.0), 0);
        assert_eq!(light_byte(1.0), 255);
        assert_eq!(light_byte(0.5), 128);
        assert_eq!(light_byte(0.127_118_65), 32);
        assert_eq!(light_byte(f32::NAN), 0);
        // Out of range wraps to the low byte, or gives 0 past 64 bits.
        assert_eq!(light_byte(2.0), 254);
        assert_eq!(light_byte(f32::MAX), 0);
        assert_eq!(light_byte(f32::MIN), 0);
    }

    #[test]
    fn arrays_shorter_than_their_size_are_an_error() {
        let mut world = World::empty();
        let full = FULL * FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(id, array(stride, full * stride));
        }
        let lot = Lot::from_world(&world).expect("full arrays read");
        assert_eq!(lot.built_center(), [512, 512]);
        world.arrays.insert(2, array(8, 0));
        assert!(Lot::from_world(&world).is_err());
        world.arrays.insert(2, array(8, full * 8 + 1));
        assert!(Lot::from_world(&world).is_err());
    }
}
