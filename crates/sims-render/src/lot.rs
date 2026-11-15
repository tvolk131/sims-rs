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
    /// The two triangles of each tile a diagonal wall splits, by level (1
    /// or 2), column and row: which triangle (by the corner it holds: 1
    /// south-west, 2 south-east, 3 north-east, 4 north-west) and its room.
    pub halves: BTreeMap<[u8; 3], [(u8, i16); 2]>,
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
            .map(|(&id, r)| {
                // A room the room update has never counted has no light
                // yet (every counted room has at least one tile). Draw
                // the outdoors in daylight then, so a house read without
                // a load still shows; indoors keeps its light of 0.
                let light = if r.half_tiles == 0 && r.outdoors {
                    1.0
                } else {
                    r.shown_light().to_f32()
                };
                (id, light_byte(light))
            })
            .collect();
        let halves = world
            .rooms
            .halves
            .iter()
            .map(|(&tile, h)| (tile, h.map(|h| (h.triangle, h.room))))
            .collect();
        Ok(Self {
            size,
            heights,
            grass,
            levels: [level(0)?, level(1)?],
            outdoor_rooms,
            room_lights,
            halves,
        })
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

    /// Whether the room of tile (x, y) on a level is outdoors. Tiles off
    /// the lot, tiles in no room, and lots whose rooms are not worked out
    /// count as outdoors.
    pub fn outdoors(&self, level: usize, x: i32, y: i32) -> bool {
        if !self.contains(x, y) {
            return true;
        }
        let Some(rooms) = &self.levels[level].rooms else {
            return true;
        };
        let room = rooms[Self::cell(x as usize, y as usize)];
        room <= 0 || self.outdoor_rooms.contains(&room)
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

    /// The light byte at a fine position on a level (0 or 1): its room's
    /// light. Off the lot and outside every room it is full light.
    pub fn light_at(&self, level: usize, fine: [i32; 2]) -> u8 {
        self.room_at(level, fine)
            .and_then(|room| self.room_lights.get(&room).copied())
            .unwrap_or(255)
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
