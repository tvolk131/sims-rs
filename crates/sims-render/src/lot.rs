//! The lot's terrain, floors and walls, read from the world's arrays.
//!
//! A [`sims_sim::World`] holds them either as the house file saves them
//! (`World::from_house`: a floor is one byte, a wall cell 8 bytes) or as
//! a running lot holds them (after `World::read_arrays`: a floor is a
//! 16-bit word, a wall cell 14 bytes with 16-bit styles and patterns). This
//! module reads both and never writes the world.

use crate::{Error, Result};
use sims_sim::World;

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
        Ok(Self {
            size,
            heights,
            grass,
            levels: [level(0)?, level(1)?],
            outdoor_rooms,
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
