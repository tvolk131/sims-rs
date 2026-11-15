//! The pools of light a lit lamp throws on the floor around it, read from
//! the game's `GameData/lightMap.dat`.
//!
//! The original loads the file at start-up (0x4b5700) into four parts, in
//! this order:
//!
//! 1. one light field per light distance `i` (0 to 3), `64 * m_i` by 232
//!    bytes (0x49be78);
//! 2. the wall tables, per kind (0, then 1), per zoom (far to near) and
//!    per light distance: `2 * m_i - 1` images of a wall's size at that
//!    zoom (0x4bb4a2, 0x4bb7c5);
//! 3. one pool map per zoom, far to near (0x4a35f0, 0x4a3877).
//!
//! The count of light distances and each `m_i` come from the light radius
//! of 12 tiles (f32 0x6f016c): four distances (0x49bdf6), and `m_i` is
//! `ceil((sqrt(4 * 12^2 - (18i + 9)(2i + 1)) + 3) / 6)` (0x49be1f): 5, 5,
//! 4 and 3. With these the parts add up to the file's size exactly, which
//! is how the pool maps' sizes were worked out: each map is a 7 by 7 tile
//! diamond, `7 * tile width` across and half that high. Each byte is the
//! light the pool adds there, 255 at the middle falling to 0 at the rim.
//!
//! Only the pool maps are used here; the light fields and wall tables
//! are not read.

/// How many tiles a pool map reaches from its middle tile, each way: its
/// width over the tile width, less one, halved (0x4a3805).
pub const REACH: i32 = 3;

/// The light distances and each one's `m_i` (see the module comment).
const DISTANCES: [usize; 4] = [5, 5, 4, 3];

/// The size of a light field: `64 * m` by 232 bytes (0x49be78).
fn field(m: usize) -> usize {
    64 * m * 232
}

/// The size of one wall table image at zoom `z` (1 far to 3 near):
/// `(0x3a << (z - 1)) + (8 << z)` rows of `16 << (z - 1)` bytes
/// (0x4bb7c5).
fn wall_image(z: usize) -> usize {
    ((0x3a << (z - 1)) + (8 << z)) * (16 << (z - 1))
}

/// A pool map's width at zoom `z` (1 far to 3 near): seven tiles.
fn pool_width(z: usize) -> usize {
    (2 * REACH as usize + 1) * (32 << (z - 1))
}

/// The three pool maps, far to near.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pools {
    maps: [Vec<u8>; 3],
}

impl Pools {
    /// Reads the pool maps out of the bytes of `lightMap.dat`, or None if
    /// its size is not the one the layout gives.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let fields: usize = DISTANCES.iter().map(|&m| field(m)).sum();
        let tables: usize = 2
            * (1..=3)
                .map(|z| {
                    DISTANCES
                        .iter()
                        .map(|&m| (2 * m - 1) * wall_image(z))
                        .sum::<usize>()
                })
                .sum::<usize>();
        let sizes = [1, 2, 3].map(|z| pool_width(z) * pool_width(z) / 2);
        if bytes.len() != fields + tables + sizes.iter().sum::<usize>() {
            return None;
        }
        let mut at = fields + tables;
        let maps = sizes.map(|size| {
            let map = bytes[at..at + size].to_vec();
            at += size;
            map
        });
        Some(Self { maps })
    }

    /// The light the pool map of zoom `zoom` (0 far to 2 near) adds at
    /// pixel (`x`, `y`) of the map, 0 off it.
    pub fn at(&self, zoom: usize, x: i32, y: i32) -> u8 {
        let Some(map) = self.maps.get(zoom) else {
            return 0;
        };
        let width = pool_width(zoom + 1) as i32;
        if x < 0 || y < 0 || x >= width || y >= width / 2 {
            return 0;
        }
        map[(y * width + x) as usize]
    }

    /// Where, in the pool map of a lamp at view offset `[a, b]` tiles from
    /// a floor tile, the tile's image starts: the map's middle tile moved
    /// by the offset on screen (0x4a3813). `tile` is the zoom's tile width.
    pub fn origin(tile: i32, [a, b]: [i32; 2]) -> [i32; 2] {
        [
            (2 * REACH + a - b) * tile / 2,
            (2 * REACH + a + b) * tile / 4,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file of the right size whose pool maps hold `pool` at every
    /// byte, and zero elsewhere.
    fn file(pool: u8) -> Vec<u8> {
        let mut bytes = vec![0; 1_744_256];
        bytes.extend(std::iter::repeat_n(pool, 526_848));
        bytes
    }

    #[test]
    fn the_layout_adds_up_to_the_games_file() {
        assert_eq!(file(0).len(), 2_271_104);
        let pools = Pools::parse(&file(9)).expect("the size fits");
        assert_eq!(pools.at(0, 0, 0), 9);
        assert_eq!(pools.at(2, 895, 447), 9);
        assert_eq!(pools.at(2, 896, 0), 0);
        assert_eq!(pools.at(0, 0, 112), 0);
        assert_eq!(pools.at(3, 0, 0), 0);
        assert!(Pools::parse(&file(0)[1..]).is_none());
    }

    #[test]
    fn a_tile_finds_its_window_by_its_screen_offset() {
        // The lamp's own tile is the middle of the map.
        assert_eq!(Pools::origin(128, [0, 0]), [384, 192]);
        // A tile one step along view x is half a tile right and a quarter
        // down; one along view y, half a tile left and a quarter down.
        assert_eq!(Pools::origin(128, [1, 0]), [448, 224]);
        assert_eq!(Pools::origin(128, [0, 1]), [320, 224]);
        assert_eq!(Pools::origin(32, [-3, -3]), [96, 0]);
    }
}
