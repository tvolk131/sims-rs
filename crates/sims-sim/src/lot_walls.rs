//! Original column-major wall cells and native collision queries. These retain
//! diagonal/style semantics independently of the authored-room topology.
use crate::{Point, Rect};
use serde::{Deserialize, Serialize};
use sims_formats::lot::WallTile;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Expanded {
    pub segments: u8,
    pub flags: u8,
    /// W, N, E, S, descending diagonal, ascending diagonal. E/S styles are
    /// uninitialized in native storage when the neighboring owner is absent.
    pub styles: [Option<u16>; 6],
    pub patterns: [u16; 8],
    pub adjacent: u8,
}

pub struct Grid<'a> {
    size: usize,
    cells: &'a [WallTile],
}

/// 0x476f5a. Object wall-placement requirements, separate from rectangle
/// collision. Successful checks preserve the caller's diagnostic error word.
pub fn placement_fit(
    wall: &Expanded,
    facing: i16,
    mask: u16,
    error: i32,
) -> Result<crate::lot_acceptance::Outcome, String> {
    use crate::lot_acceptance::Outcome;
    if !(0..=7).contains(&facing) {
        return Err("wall-fit direction outside original rotation table".into());
    }
    let reject = |error| {
        Ok(Outcome {
            accepted: false,
            error,
        })
    };
    let diagonal = wall.segments & 48 != 0;
    if mask & 0x40 != 0 && !diagonal {
        return reject(23);
    }
    if mask & 0x80 == 0 && diagonal {
        return reject(7);
    }
    if mask == 0 {
        return Ok(Outcome {
            accepted: true,
            error,
        });
    }
    let r = ((-i32::from(facing) & 7) >> 1) as usize;
    let mut bits = wall.segments;
    let mut styles = wall.styles;
    if r != 0 {
        bits = 0;
        let mut bit = 1u8;
        while bit <= 32 && wall.segments & bit == 0 {
            bit <<= 1;
        }
        while bit <= 32 {
            if wall.segments & bit != 0 {
                let rotated = if bit < 16 {
                    ((bit << r) | (bit >> (4 - r))) & 15
                } else if r & 1 == 0 {
                    bit
                } else {
                    bit ^ 48
                };
                bits |= rotated;
                if bit < 16 {
                    styles[rotated.trailing_zeros() as usize] =
                        wall.styles[bit.trailing_zeros() as usize];
                }
            }
            bit <<= 1;
            // Native 0x53bc11 handles bit32 only when it was the first bit.
            if bit >= 32 {
                break;
            }
        }
    }
    let corner = bits & 1 != 0 && bits & 10 != 0 || bits & 4 != 0 && bits & 10 != 0;
    if mask & 0x20 != 0 && !corner {
        return reject(4);
    }
    if mask & 0x10 != 0 && corner {
        return reject(5);
    }
    for (bit, required, forbidden) in [(2, 1, 0x100), (1, 8, 0x800), (4, 2, 0x200), (8, 4, 0x400)] {
        let present = bits & bit != 0;
        if mask & forbidden != 0 && present {
            return reject(7);
        }
        if mask & required != 0 {
            if !present {
                return reject(6);
            }
            let style = styles[bit.trailing_zeros() as usize].ok_or("wall-fit style absent")?;
            if matches!(style, 2 | 12 | 13 | 14) {
                return reject(22);
            }
        }
    }
    Ok(Outcome {
        accepted: true,
        error,
    })
}
impl<'a> Grid<'a> {
    pub fn new(size: usize, cells: &'a [WallTile]) -> Result<Self, String> {
        if !(1..=64).contains(&size) || cells.len() != size * size {
            return Err("invalid original wall grid dimensions".into());
        }
        if cells.iter().any(|t| t.segments & !0x3f != 0) {
            return Err("unsupported original wall segment bits".into());
        }
        Ok(Self { size, cells })
    }
    pub fn cell(&self, x: usize, y: usize) -> Option<&WallTile> {
        (x < self.size && y < self.size).then(|| &self.cells[x * self.size + y])
    }
    /// 0x4795c5 / 0x53b98f. A cell owns its W/N styles; E/S come from the
    /// neighboring owner. The two diagonals share the cell's second style.
    pub fn expanded(&self, x: usize, y: usize) -> Result<Expanded, String> {
        let t = self.cell(x, y).ok_or("wall query outside lot")?;
        let east = self.cell(x + 1, y).filter(|c| c.segments & 1 != 0);
        let south = self.cell(x, y + 1).filter(|c| c.segments & 2 != 0);
        let mut flags = t.flags;
        if let Some(e) = east {
            flags = (flags & !0x30) | (e.flags & 3) << 4;
        }
        if let Some(s) = south {
            flags = (flags & !0xc0) | (s.flags & 0xc) << 4;
        }
        let diagonal = |x, y| self.cell(x, y).is_some_and(|c| c.segments & 0x30 != 0);
        let adjacent = u8::from(diagonal(x, y + 1))
            | (u8::from(diagonal(x + 1, y)) << 1)
            | (u8::from(x > 0 && diagonal(x - 1, y)) << 2)
            | (u8::from(y > 0 && diagonal(x, y - 1)) << 3);
        Ok(Expanded {
            segments: t.segments,
            flags,
            styles: [
                Some(t.styles[0]),
                Some(t.styles[1]),
                east.map(|c| c.styles[0]),
                south.map(|c| c.styles[1]),
                Some(t.styles[1]),
                Some(t.styles[1]),
            ],
            patterns: [
                t.patterns[0],
                t.patterns[1],
                t.patterns[3],
                t.patterns[2],
                if t.segments & 16 != 0 {
                    t.patterns[2]
                } else {
                    0
                },
                if t.segments & 16 != 0 {
                    t.patterns[3]
                } else {
                    0
                },
                if t.segments & 32 != 0 {
                    t.patterns[2]
                } else {
                    0
                },
                if t.segments & 32 != 0 {
                    t.patterns[3]
                } else {
                    0
                },
            ],
            adjacent,
        })
    }
    /// Native 0x476dbc, using Rust plan coordinates (native physical Y/X).
    /// Exact contact does not cross a wall. Styles 3/5/6/15/23 do not block.
    pub fn blocks_bounds(&self, b: Rect) -> Result<bool, String> {
        let extent = self.size as i32 * 16;
        if b.min.x < 0
            || b.min.y < 0
            || b.max.x > extent
            || b.max.y > extent
            || b.min.x > b.max.x
            || b.min.y > b.max.y
        {
            return Err("wall collision bounds outside lot".into());
        }
        for row in b.min.y >> 4..=(b.max.y - 1) >> 4 {
            for col in b.min.x >> 4..=(b.max.x - 1) >> 4 {
                let t = self
                    .cell(col as usize, row as usize)
                    .ok_or("wall cell outside lot")?;
                // Native processes the first set bit even when it is 0x20,
                // but its continuation loop stops before processing bit 0x20.
                let mask = if t.segments == 0x20 {
                    0x20
                } else {
                    t.segments & 0x1f
                };
                for bit in [1, 2, 16, 32] {
                    let style = t.styles[usize::from(bit != 1)];
                    if mask & bit == 0 || matches!(style, 3 | 5 | 6 | 15 | 23) {
                        continue;
                    }
                    let (lo, hi, line) = match bit {
                        1 => (b.min.x, b.max.x, col * 16),
                        2 => (b.min.y, b.max.y, row * 16),
                        16 => (b.min.x + b.min.y, b.max.x + b.max.y, (col + row + 1) * 16),
                        32 => (b.min.y - b.max.x, b.max.y - b.min.x, (row - col) * 16),
                        _ => unreachable!(),
                    };
                    if (lo < line && line < hi) || (hi < line && line < lo) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    /// Endpoints of owned segments, in plan coordinates. Mirrored E/S bits
    /// are omitted because their neighboring W/N owner supplies the edge.
    pub fn owned_segments(&self) -> impl Iterator<Item = (Point, Point, u16)> + '_ {
        self.cells.iter().enumerate().flat_map(move |(i, t)| {
            let x = (i / self.size) as i32 * 16;
            let y = (i % self.size) as i32 * 16;
            [
                (1, Point::new(x, y), Point::new(x, y + 16), t.styles[0]),
                (2, Point::new(x, y), Point::new(x + 16, y), t.styles[1]),
                (
                    16,
                    Point::new(x + 16, y),
                    Point::new(x, y + 16),
                    t.styles[1],
                ),
                (
                    32,
                    Point::new(x, y),
                    Point::new(x + 16, y + 16),
                    t.styles[1],
                ),
            ]
            .into_iter()
            .filter_map(move |(mask, a, b, style)| {
                (t.segments & mask != 0).then_some((a, b, style))
            })
        })
    }
}
