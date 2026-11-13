//! Original room-to-rectangle conversion. Coordinates use lot column/row order.
//!
//! `prepare` corresponds to 0x4818af up to 0x4821c4. Its rectangles are
//! intermediate data; `geometry::finalize` implements the separately verified
//! hard-prefix transformation. Full routing wrapper parity remains unverified.
use crate::{lot_walls::Grid, regions::Region};
use serde::{Deserialize, Serialize};
use sims_formats::lot::WallTile;
pub mod animator;
pub mod candidate;
pub mod completion;
pub mod departure;
pub mod geometry;
pub mod live;
pub mod obstacle;
pub mod path;
pub mod portal;
pub mod refresh;
pub mod retry;
pub mod search;
pub mod seat;
pub mod shoo;

/// Retained world+10 owns native bounds. Historical inferred sessions without
/// resize history retain their old extent; global23 is never a substitute.
pub(crate) fn active_size(world: &sims_vm::World, backing: usize) -> Result<usize, String> {
    world
        .map_size
        .map_or(Ok(backing), |m| m.checked_size(backing))
}

/// World services resolved before evaluating 0x4821f3. `partner_room` is the
/// result of original portal-part traversal, not any neighboring room guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileOccupant {
    pub kind: i16,
    pub flags: u16,
    pub function15: bool,
    pub portal_class: bool,
    pub partner_room: Option<i16>,
}

/// Original 0x4821f3 with the tile's object iterator supplied by the world.
/// Water is Arry 10/world+0x64, distinct from the pool region map (Arry 9).
pub fn tile_available(
    level: i32,
    primary: i16,
    room: i16,
    water: u8,
    occupants: &[TileOccupant],
) -> Result<bool, String> {
    let mut connected = primary == room || primary == -5;
    if !connected {
        // Native enumeration continues after a match. A malformed subsequent
        // portal must not be hidden by a successful earlier match.
        for o in occupants {
            if o.kind == 8 && o.function15 && o.portal_class {
                connected |= o
                    .partner_room
                    .ok_or("route portal has no resolved partner")?
                    == room;
            }
        }
    }
    Ok(
        connected
            && (level != 1 || water == 255 || occupants.iter().any(|o| o.flags & 0x1000 != 0)),
    )
}

pub struct Input<'a> {
    pub size: usize,
    pub level: i32,
    pub room: i16,
    pub primary: &'a [i16],
    pub walls: &'a [WallTile],
    pub coverings: &'a [u16],
    pub flags: &'a [u8],
    /// Results of 0x4821f3 for this room, including portals and water rules.
    pub available: &'a [bool],
    pub outside: bool,
    /// The caller requests terrain costs and the lot-type query returns 1.
    pub terrain: bool,
    /// Original global 0x6e4ad4: suppress the ground flag 0x20 cost marker.
    pub ignore_marker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prepared {
    pub rectangles: Vec<Region>,
    /// Only this prefix receives the final seven-unit inflation.
    pub hard_count: usize,
}

pub fn prepare(c: Input<'_>) -> Result<Prepared, String> {
    if !(3..=64).contains(&c.size)
        || !(1..=2).contains(&c.level)
        || c.room < 0
        || i32::from((c.room as u16 & 0xc00) >> 10) + 1 != c.level
        || [
            c.primary.len(),
            c.coverings.len(),
            c.flags.len(),
            c.available.len(),
        ]
        .into_iter()
        .any(|len| len != c.size * c.size)
    {
        return Err("invalid original routing grid".into());
    }
    let grid = Grid::new(c.size, c.walls)?;
    let index = |x: usize, y: usize| x * c.size + y;
    let available = |x, y| c.available[index(x, y)];
    let covered = |x, y| c.coverings[index(x, y)] != 0 && available(x, y);
    let mut bounds = None;
    for y in 1..c.size - 1 {
        for x in 1..c.size - 1 {
            if c.primary[index(x, y)] == c.room {
                let b = bounds.get_or_insert([x, y, x, y]);
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
        }
    }
    // The original assumes a primary cell exists. Split-only rooms cannot
    // safely borrow a neighboring room's bounds or an invented whole-lot box.
    let [left, top, right, bottom] = bounds.ok_or("room has no primary routing cell")?;
    let (left, top) = (left.saturating_sub(1).max(1), top.saturating_sub(1).max(1));
    let (right, bottom) = ((right + 1).min(c.size - 2), (bottom + 1).min(c.size - 2));
    let rect = |a, b, d, e, cost| {
        Region([
            a as i32 * 16,
            b as i32 * 16,
            d as i32 * 16,
            e as i32 * 16,
            cost,
        ])
    };
    let mut rectangles = vec![
        rect(left - 1, top - 1, left, bottom + 2, i32::MAX),
        rect(right + 1, top - 1, right + 2, bottom + 2, i32::MAX),
        rect(left - 1, top - 1, right + 2, top, i32::MAX),
        rect(left - 1, bottom + 1, right + 2, bottom + 2, i32::MAX),
    ];
    // Native first-overlap order includes zero-area wall and diagonal seeds.
    let overlap = |rs: &[Region], seed: Region| rs.iter().find(|r| seed.overlaps(**r)).copied();
    let grow = |x: usize, y: usize, accepts: &dyn Fn(usize, usize) -> bool| {
        let mut end_x = x + 1;
        while end_x <= right && accepts(end_x, y) {
            end_x += 1;
        }
        let mut end_y = y + 1;
        // The native loop queries the row past the bound before its limit test.
        while (x..end_x).all(|xx| accepts(xx, end_y)) && end_y <= bottom {
            end_y += 1;
        }
        (end_x, end_y)
    };
    for y in top..=bottom {
        let mut x = left;
        while x <= right {
            if !available(x, y) {
                let seed = rect(x, y, x + 1, y + 1, i32::MAX);
                if let Some(r) = overlap(&rectangles, seed) {
                    let end = r.0[2] as usize / 16;
                    if end != x {
                        x = end - 1;
                    }
                } else {
                    let (end_x, end_y) = grow(x, y, &|xx, yy| !available(xx, yy));
                    rectangles.push(rect(x, y, end_x, end_y, i32::MAX));
                }
            } else {
                let w = grid.expanded(x, y)?;
                if (w.segments & 8 != 0 && w.styles[3].is_none())
                    || (w.segments & 4 != 0 && w.styles[2].is_none())
                {
                    return Err("route wall mirror has no owning segment".into());
                }
                let blocking = |style| !matches!(style, Some(3 | 5 | 6 | 15 | 23));
                if w.segments & 8 != 0 && blocking(w.styles[3]) && available(x, y + 1) {
                    rectangles.push(rect(x, y + 1, x + 1, y + 1, i32::MAX));
                }
                if w.segments & 4 != 0 && blocking(w.styles[2]) && available(x + 1, y) {
                    rectangles.push(rect(x + 1, y, x + 1, y + 1, i32::MAX));
                }
                if w.segments & 0x30 != 0 {
                    let (a, b) = if w.segments & 16 != 0 {
                        (12, 4)
                    } else {
                        (4, 12)
                    };
                    let (xx, yy) = (x as i32 * 16, y as i32 * 16);
                    // Two 8x8 quadrant rectangles shrink by four on every side.
                    // geometry::finalize later inflates these point seeds.
                    rectangles.push(Region([xx + 4, yy + a, xx + 4, yy + a, i32::MAX]));
                    rectangles.push(Region([xx + 12, yy + b, xx + 12, yy + b, i32::MAX]));
                }
            }
            x += 1;
        }
    }
    let hard_count = rectangles.len();
    if c.terrain && c.level == 1 && c.outside {
        for y in top..=bottom {
            let mut x = left;
            while x <= right {
                // Seed selection checks the covering only. Eligibility is used
                // during growth; hard rectangles already cover unavailable cells.
                if c.coverings[index(x, y)] != 0 {
                    let cost = if !c.ignore_marker && c.flags[index(x, y)] & 0x20 != 0 {
                        10
                    } else {
                        0
                    };
                    let seed = rect(x, y, x + 1, y + 1, cost);
                    if let Some(r) = overlap(&rectangles, seed) {
                        let end = r.0[2] as usize / 16;
                        if end != x {
                            x = end - 1;
                        }
                    } else {
                        let (end_x, end_y) = grow(x, y, &covered);
                        // A merged patch retains its seed's cost, even when the
                        // marker changes in later cells of the patch.
                        rectangles.push(rect(x, y, end_x, end_y, cost));
                    }
                }
                x += 1;
            }
        }
    }
    Ok(Prepared {
        rectangles,
        hard_count,
    })
}

pub mod movement;
