//! The pools of light a lit lamp throws on the floor and walls around it,
//! read from the game's `GameData/lightMap.dat`.
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
//! The pool maps light floors; the wall tables light walls (see
//! [`Pools::wall`]). The light fields are not read.
//!
//! # Walls
//!
//! A straight wall takes the pools of the lamps marked on its tile in the
//! view: the tile whose low-x edge (a wall along view y, direction 1) or
//! low-y edge (along view x, direction 2) it stands on, the side the
//! camera sees (0x4aed23, called from the wall pass 0x53db67). Of the
//! tile's two lamps, each on the seen side of the wall (a view offset
//! from the lamp to the tile of at most 0 across the wall) is kept, as
//! `along` and `out` tiles: for direction 1 `-dy` and `-dx`, for
//! direction 2 `dx` and `-dy` (0x4c78c5). A lamp is used if its table has
//! an image for it: `out` below 4 and `along` within `m_out - 1` either
//! way (0x4b5c98). Diagonal walls take no pools. The image is direction
//! 1's table (kind 0) or direction 2's (kind 1), at the zoom, for `out`,
//! image `along + m_out - 1` (0x4b5c0c, 0x4bb595). It is a wall's face,
//! as wide as the wall sprite, and is read row by row with the sprite,
//! the read cursor starting [`wall_rise`] rows before the table, so the
//! image's first row is that many rows below the sprite's top (0x4c7cd4
//! to 0x4c7cf8; the rise is the table's `1 << zoom`, 0x4bb4a2). A
//! corner's cap takes pools the same way. Each pixel
//! of the wall then takes the drawn light byte plus each lamp's byte
//! there, kept to 255, as a floor does (0x4b7630 for one lamp, 0x4b7420
//! for two). That the image lines up with our wall sprites this way is
//! checked: the face it covers is the sprites' opaque face, at every zoom
//! and both directions.

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

/// The size of one wall table image at zoom index `zoom` (0 far to 2
/// near): width and height.
fn wall_size(zoom: usize) -> (usize, usize) {
    let z = zoom + 1;
    (16 << (z - 1), (0x3a << (z - 1)) + (8 << z))
}

/// How many rows below a wall sprite's top its pool image starts, at
/// zoom index `zoom` (0 far to 2 near): the original's read cursor starts
/// this many rows before the table, so sprite row `r` reads table row
/// `r - rise` and the rows above are unlit by it. The table's
/// `1 << zoom`, zoom counted from 1 (0x4bb4a2).
pub fn wall_rise(zoom: usize) -> usize {
    2 << zoom
}

/// Which lamp a straight wall takes a pool from, as (`along`, `out`), for
/// a lamp at view offset `[dx, dy]` (from the lamp to the wall's tile) and
/// a wall along view y (`along_y`, direction 1) or view x (direction 2);
/// None if the lamp is behind the wall or its table has no image for it
/// (0x4c78c5, 0x4b5c98). See [the module docs](self#walls).
pub fn wall_lamp(along_y: bool, [dx, dy]: [i32; 2]) -> Option<(i32, usize)> {
    let (across, along) = if along_y {
        (dx, dy.checked_neg()?)
    } else {
        (dy, dx)
    };
    if across > 0 {
        return None;
    }
    let out = usize::try_from(across.unsigned_abs()).ok()?;
    let half = DISTANCES.get(out)? - 1;
    (along.unsigned_abs() as usize <= half).then_some((along, out))
}

/// The three pool maps, far to near, and the wall tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pools {
    maps: [Vec<u8>; 3],
    walls: Vec<u8>,
}

/// One wall table image: a wall's face, a light byte per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallPool {
    pub width: usize,
    pub height: usize,
    bytes: Vec<u8>,
}

impl WallPool {
    /// The light the image adds at pixel (`x`, `y`), 0 off it.
    pub fn at(&self, x: i64, y: i64) -> u8 {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
            return 0;
        };
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.bytes[y * self.width + x]
    }
}

/// Borrowed view of one wall table image inside [`Pools`]: avoids copying
/// up to 19 KB per lamp per wall segment on the render hot path.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WallPoolRef<'a> {
    pub width: usize,
    pub height: usize,
    bytes: &'a [u8],
}

impl WallPoolRef<'_> {
    /// The light the image adds at pixel (`x`, `y`), 0 off it.
    #[inline]
    pub(crate) fn at(&self, x: i64, y: i64) -> u8 {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
            return 0;
        };
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.bytes[y * self.width + x]
    }
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
        let walls = bytes[fields..fields + tables].to_vec();
        let mut at = fields + tables;
        let maps = sizes.map(|size| {
            let map = bytes[at..at + size].to_vec();
            at += size;
            map
        });
        Some(Self { maps, walls })
    }

    /// Borrowed slice of one wall table image ([`Pools::wall`] without copying).
    pub(crate) fn wall_ref(
        &self,
        along_y: bool,
        zoom: usize,
        along: i32,
        out: usize,
    ) -> Option<WallPoolRef<'_>> {
        let half = *DISTANCES.get(out)? - 1;
        if zoom > 2 || along.unsigned_abs() as usize > half {
            return None;
        }
        let images = |z: usize, m: usize| {
            let (w, h) = wall_size(z);
            (2 * m - 1) * w * h
        };
        let kind = usize::from(!along_y);
        let per_kind: usize = (0..3)
            .map(|z| DISTANCES.iter().map(|&m| images(z, m)).sum::<usize>())
            .sum();
        let before_zoom: usize = (0..zoom)
            .map(|z| DISTANCES.iter().map(|&m| images(z, m)).sum::<usize>())
            .sum();
        let before_out: usize = DISTANCES[..out].iter().map(|&m| images(zoom, m)).sum();
        let (width, height) = wall_size(zoom);
        let index = (along + half as i32) as usize;
        let start = kind * per_kind + before_zoom + before_out + index * width * height;
        let bytes = self.walls.get(start..start + width * height)?;
        Some(WallPoolRef {
            width,
            height,
            bytes,
        })
    }

    /// The wall table image for a lamp `along` and `out` tiles from a
    /// wall along view y (`along_y`, direction 1, kind 0) or view x
    /// (direction 2, kind 1), at zoom index `zoom` (0 far to 2 near), as
    /// [`wall_lamp`] gives them. None if the tables have no such image.
    pub fn wall(&self, along_y: bool, zoom: usize, along: i32, out: usize) -> Option<WallPool> {
        let r = self.wall_ref(along_y, zoom, along, out)?;
        Some(WallPool {
            width: r.width,
            height: r.height,
            bytes: r.bytes.to_vec(),
        })
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
    fn a_wall_takes_the_lamps_on_its_seen_side_its_tables_reach() {
        // Direction 1 (along view y): across is dx, along is -dy.
        assert_eq!(wall_lamp(true, [0, 0]), Some((0, 0)));
        assert_eq!(wall_lamp(true, [-2, 3]), Some((-3, 2)));
        assert_eq!(wall_lamp(true, [1, 0]), None);
        // Direction 2 (along view x): across is dy, along is dx.
        assert_eq!(wall_lamp(false, [3, -1]), Some((3, 1)));
        assert_eq!(wall_lamp(false, [0, 1]), None);
        // Out 0 and 1 reach 4 along, 2 reaches 3, 3 reaches 2; 4 none.
        assert_eq!(wall_lamp(true, [-1, -4]), Some((4, 1)));
        assert_eq!(wall_lamp(true, [-2, 4]), None);
        assert_eq!(wall_lamp(true, [-3, 2]), Some((-2, 3)));
        assert_eq!(wall_lamp(true, [-3, 3]), None);
        // Offsets no lamp record holds are refused, not overflowed.
        for d in [i32::MIN, i32::MAX] {
            for along_y in [true, false] {
                assert_eq!(wall_lamp(along_y, [0, d]), None);
                assert_eq!(wall_lamp(along_y, [d, 0]), None);
            }
        }
        assert_eq!(wall_lamp(true, [-4, 0]), None);
    }

    #[test]
    fn wall_images_are_found_in_the_files_order() {
        // Number each table image's bytes by its place in the file.
        let mut bytes = vec![0u8; 2_271_104];
        let fields: usize = DISTANCES.iter().map(|&m| field(m)).sum();
        let mut at = fields;
        let mut n = 0u8;
        for _kind in 0..2 {
            for z in 0..3 {
                let (w, h) = wall_size(z);
                for &m in &DISTANCES {
                    for _ in 0..2 * m - 1 {
                        bytes[at..at + w * h].fill(n);
                        at += w * h;
                        n = n.wrapping_add(1);
                    }
                }
            }
        }
        assert_eq!(at, 1_744_256);
        let pools = Pools::parse(&bytes).unwrap();
        let first = pools.wall(true, 0, -4, 0).unwrap();
        assert_eq!((first.width, first.height, first.at(0, 0)), (16, 74, 0));
        assert_eq!(pools.wall(true, 0, 4, 0).unwrap().at(15, 73), 8);
        assert_eq!(pools.wall(true, 0, 0, 1).unwrap().at(0, 0), 9 + 4);
        assert_eq!(pools.wall(true, 0, -2, 3).unwrap().at(0, 0), 9 + 9 + 7);
        // Kind 0 holds 3 zooms of 9 + 9 + 7 + 5 = 30 images.
        let near = pools.wall(true, 2, 0, 0).unwrap();
        assert_eq!((near.width, near.height, near.at(0, 0)), (64, 296, 60 + 4));
        assert_eq!(pools.wall(false, 0, -4, 0).unwrap().at(0, 0), 90);
        assert_eq!(
            pools.wall(false, 2, 2, 3).unwrap().at(63, 295),
            90 + 60 + 25 + 4
        );
        assert_eq!(first.at(16, 0), 0);
        assert_eq!(first.at(0, -1), 0);
        assert!(pools.wall(true, 0, 3, 3).is_none());
        assert!(pools.wall(true, 0, 0, 4).is_none());
        assert!(pools.wall(true, 3, 0, 0).is_none());
        assert_eq!([0, 1, 2].map(wall_rise), [2, 4, 8]);
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
