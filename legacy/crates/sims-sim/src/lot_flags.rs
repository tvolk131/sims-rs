//! Exterior and build-support passes in native 0x479d9a. Build support is not
//! an upstairs floor surface: walking still requires an actual floor covering.
use super::lot_walls::{Expanded, Grid};
use serde::{Deserialize, Serialize};
use sims_formats::lot::WallTile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rebuilt {
    pub flags: Vec<u8>,
    pub upper_contents: bool,
    pub upper_support: bool,
}

fn encloses(style: u16) -> bool {
    !matches!(style, 2 | 12 | 13 | 14)
}
fn diagonal(tile: &WallTile) -> bool {
    tile.segments & 0x30 != 0 && encloses(tile.styles[1])
}
fn edge(expanded: &Expanded, direction: u8) -> Result<bool, String> {
    if expanded.segments & direction == 0 {
        return Ok(false);
    }
    let style = expanded.styles[direction.trailing_zeros() as usize]
        .ok_or("wall edge has no owning neighbor style")?;
    Ok(encloses(style))
}

pub struct Input<'a> {
    pub size: usize,
    pub level: i32,
    pub walls: &'a [WallTile],
    pub floors: &'a [u16],
    pub object_heads: &'a [u16],
    pub heights: &'a [[u8; 4]],
    pub saved_flags: &'a [u8],
    /// Required upstairs; the completed lower floor's flags.
    pub below: Option<&'a [u8]>,
}

pub fn rebuild(input: Input<'_>) -> Result<Rebuilt, String> {
    let active = input.size;
    rebuild_in_active(input, active)
}

///479d9a masks the complete allocated flag arrays, then floods/scans only
///world+10's active extent. Array stride and active extent are distinct owners.
pub fn rebuild_in_active(input: Input<'_>, active: usize) -> Result<Rebuilt, String> {
    let Input {
        size,
        level,
        walls,
        floors,
        object_heads,
        heights,
        saved_flags,
        below,
    } = input;
    let grid = Grid::new(size, walls)?;
    let count = size * size;
    if !(1..=2).contains(&level)
        || !(2..=size).contains(&active)
        || floors.len() != count
        || object_heads.len() != count
        || heights.len() != count
        || saved_flags.len() != count
        || (level == 2 && below.is_none_or(|b| b.len() != count))
    {
        return Err("invalid native tile-flag inputs".into());
    }
    // The native buildable lot excludes its border. Diagonals there can read
    // neighboring rows outside the allocated map; do not manufacture a result.
    for x in 0..active {
        for y in 0..active {
            if (x == 0 || y == 0 || x + 1 == size || y + 1 == size)
                && diagonal(&walls[x * size + y])
            {
                return Err("solid diagonal on original lot border is unsupported".into());
            }
        }
    }
    // Native masks with 0x28 (not boolean true as the decompiler suggests):
    // retain existing support/lot markers, clear buildability and exterior.
    let mut flags: Vec<_> = saved_flags.iter().map(|f| f & 0x28).collect();
    let mut stack = vec![(0usize, 0usize)];
    let neighbor = |x: usize, y: usize, dx: i32, dy: i32| {
        let x = x as i32 + dx;
        let y = y as i32 + dy;
        (x >= 0 && y >= 0 && x < active as i32 && y < active as i32)
            .then_some((x as usize, y as usize))
    };
    while let Some((x, y)) = stack.pop() {
        let cell = x * size + y;
        if flags[cell] & 2 != 0 {
            continue;
        }
        let mut mark = flags[cell] | 6;
        if !diagonal(&walls[cell]) {
            let expanded = grid.expanded(x, y)?;
            for (dx, dy, direction) in [(-1, 0, 1), (1, 0, 4), (0, -1, 2), (0, 1, 8)] {
                if let Some((nx, ny)) = neighbor(x, y, dx, dy) {
                    let n = nx * size + ny;
                    if flags[n] & 2 == 0 && !edge(&expanded, direction)? && !diagonal(&walls[n]) {
                        stack.push((nx, ny));
                    }
                }
            }
        } else {
            let descending = walls[cell].segments & 16 != 0;
            let dy = if descending { 1 } else { -1 };
            mark |= if descending { 64 } else { 128 };
            for (a, b) in [((0, dy), (1, 0)), ((-1, 0), (0, -dy))] {
                let a = neighbor(x, y, a.0, a.1);
                let b = neighbor(x, y, b.0, b.1);
                let visited =
                    |p: Option<(usize, usize)>| p.is_none_or(|(x, y)| flags[x * size + y] & 2 != 0);
                match (visited(a), visited(b)) {
                    (true, false) => stack.push(b.unwrap()),
                    (false, true) => stack.push(a.unwrap()),
                    _ => (),
                }
            }
        }
        flags[cell] = mark;
    }
    let mut upper_support = false;
    for x in 1..active.saturating_sub(1) {
        for y in 1..active.saturating_sub(1) {
            let cell = x * size + y;
            if heights[cell].iter().any(|h| *h != heights[cell][0]) {
                continue;
            }
            let supported = if level == 1 {
                true
            } else {
                let below = below.unwrap();
                if below[cell] & 4 == 0 || below[cell] & 8 != 0 {
                    true
                } else {
                    let has_neighbor_floor = [(0, -1), (-1, 0), (1, 0), (0, 1)]
                        .into_iter()
                        .filter_map(|(dx, dy)| neighbor(x, y, dx, dy))
                        .any(|(nx, ny)| floors[nx * size + ny] != 0);
                    has_neighbor_floor
                        && (-2i32..=2).any(|dy| {
                            (-2i32..=2).any(|dx| {
                                if dx == 0 && dy == 0 {
                                    return false;
                                }
                                neighbor(x, y, dx, dy).is_some_and(|(nx, ny)| {
                                    let b = below[nx * size + ny];
                                    b & 4 == 0 || (b & 8 != 0 && dx.abs() <= 1 && dy.abs() <= 1)
                                })
                            })
                        })
                }
            };
            if supported {
                flags[cell] |= 1;
                upper_support |= level == 2;
            }
        }
    }
    Ok(Rebuilt {
        flags,
        upper_contents: level == 2
            && (0..active * active).any(|n| {
                let cell = n / active * size + n % active;
                floors[cell] != 0 || object_heads[cell] != 0 || walls[cell].segments != 0
            }),
        upper_support,
    })
}
