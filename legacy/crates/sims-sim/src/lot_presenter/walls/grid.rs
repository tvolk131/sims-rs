//! Corner-grid production (479c58 within479d9a) and the tables initialized by
//! 53d2fc. These are derived from the loaded wall cells, before edge allocation.
use crate::lot_walls::Grid;
use sims_formats::lot::WallTile;

/// The outermost ring is excluded by the original getter even when its saved
/// wall bytes are nonzero. Coordinates and output storage are column-major.
pub fn corners(size: usize, walls: &[WallTile]) -> Result<Vec<u8>, String> {
    let grid = Grid::new(size, walls)?;
    let cell = |x: usize, y: usize| {
        if x == 0 || y == 0 || x >= size - 1 || y >= size - 1 {
            0
        } else {
            grid.cell(x, y).map_or(0, |w| w.segments)
        }
    };
    let mut result = vec![0; size * size];
    for x in 0..size {
        for y in 0..size {
            let nw = cell(x.wrapping_sub(1), y.wrapping_sub(1));
            let n = cell(x, y.wrapping_sub(1));
            let w = cell(x.wrapping_sub(1), y);
            let here = cell(x, y);
            result[x * size + y] = u8::from(n & 1 != 0)
                | (u8::from(nw & 8 != 0) << 1)
                | (u8::from(w & 4 != 0) << 2)
                | (u8::from(here & 2 != 0) << 3)
                | (u8::from(nw & 32 != 0) << 4)
                | (u8::from(w & 16 != 0) << 5)
                | (u8::from(here & 32 != 0) << 6)
                | (u8::from(n & 16 != 0) << 7);
        }
    }
    Ok(result)
}

/// 53d4c4,53d4ec,53d2fc: (junction, endpoint) tables. Preserve the asymmetric
/// invalid-junction test and duplicated bit8 test in the original initializer.
pub fn classification(mask: u8) -> (bool, bool) {
    let valid = !(mask & 16 != 0 && mask & 3 != 0
        || mask & 32 != 0 && mask & 6 != 0
        || mask & 64 != 0 && mask & 4 != 0
        || mask & 128 != 0 && mask & 9 != 0);
    let junction = mask & 1 != 0 && mask & 0xfa != 0
        || mask & 2 != 0 && mask & 0xf5 != 0
        || mask & 8 != 0 && mask & 0xf5 != 0
        || mask & 16 != 0 && mask & 0xaf != 0
        || mask & 64 != 0 && mask & 0xaf != 0
        || mask & 128 != 0 && mask & 0x5f != 0
        || mask & 32 != 0 && mask & 0x5f != 0;
    (valid && junction, valid && mask.count_ones() == 1)
}

/// Nonzero rotation rows of 53d2fc. Row zero of the native array is the
/// junction predicate, not an identity rotation; native callers skip it.
pub fn rotate(mask: u8, rotation: u8) -> Result<u8, String> {
    let shift = match rotation {
        0 => return Ok(mask),
        1..=3 => 4 - rotation,
        _ => return Err("wall-corner rotation outside original table".into()),
    };
    let nibble = |n: u8| ((n << shift) | (n >> (4 - shift))) & 15;
    Ok(nibble(mask & 15) | (nibble(mask >> 4) << 4))
}
