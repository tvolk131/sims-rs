//! Rectangles and the growth of a free rectangle among obstacles
//! (0x469b26), the building block of the planner's graph.
//!
//! Ported from the old prototype (legacy/crates/sims-sim/src/regions.rs:
//! `Region`, `Space::obstacle`, `Space::expand`), which compared the
//! growth with the original's code run alone. The aspect limit that
//! routine also has is left out: the planner never turns it on.

/// A rectangle with a cost: least first and second coordinates, then the
/// greatest, then the cost of crossing it. A cost of `i32::MAX` is a hard
/// obstacle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region(pub [i32; 5]);

/// The cost of a rectangle nothing may cross.
pub(crate) const HARD: i32 = i32::MAX;

impl Region {
    /// Whether two rectangles overlap. A rectangle of no width or height
    /// overlaps one it lies strictly inside.
    pub fn overlaps(self, other: Self) -> bool {
        let a = self.0;
        let b = other.0;
        a[1] < b[3] && b[1] < a[3] && a[0] < b[2] && b[0] < a[2]
    }

    /// Grows the rectangle by `by` on every side (Wine's InflateRect, with
    /// 32-bit wrapping).
    pub fn inflate(self, by: i32) -> Self {
        let [left, top, right, bottom, cost] = self.0;
        Self([
            left.wrapping_sub(by),
            top.wrapping_sub(by),
            right.wrapping_add(by),
            bottom.wrapping_add(by),
            cost,
        ])
    }
}

/// Grid resolution: 16x16 coarse buckets of 64x64 fine units (4x4 tiles).
/// Coordinates outside `0..1024` clamp into the edge buckets so negative,
/// border-inflated, and oversized rectangles remain covered identically to a
/// full linear scan.
const GRID_DIM: usize = 16;
const GRID_CELLS: usize = GRID_DIM * GRID_DIM;
const CELL_SHIFT: u32 = 6; // 64 fine units = 4 tiles

#[inline]
fn cell_coord(value: i32) -> usize {
    (value >> CELL_SHIFT).clamp(0, (GRID_DIM - 1) as i32) as usize
}

/// Returns the inclusive bucket range on one axis for `[lo, hi)`.
///
/// Handles standard (`lo < hi`), zero-width (`lo == hi`), and inverted
/// (`lo > hi`) rectangles so any pair `(a, b)` with `a.overlaps(b)` is
/// guaranteed to share at least one cell.
#[inline]
fn axis_cells(lo: i32, hi: i32) -> (usize, usize) {
    if lo < hi {
        (cell_coord(lo), cell_coord(hi - 1))
    } else {
        (cell_coord(hi), cell_coord(lo))
    }
}

/// Coarse 4x4-tile spatial index storing ascending rectangle indices per cell.
#[derive(Debug, Clone)]
pub(crate) struct SpatialGrid {
    cells: [Vec<u32>; GRID_CELLS],
}

impl SpatialGrid {
    pub fn new() -> Self {
        Self {
            cells: std::array::from_fn(|_| Vec::new()),
        }
    }

    pub fn from_obstacles(dynamic: &[Region], fixed: &[Region]) -> Self {
        let mut grid = Self::new();
        for (i, &rect) in dynamic.iter().chain(fixed).enumerate() {
            grid.insert(i, rect);
        }
        grid
    }

    /// Inserts `index` into every cell touched by `rect`. Callers must insert
    /// indices in strictly ascending order so every cell bucket stays sorted.
    #[inline]
    pub fn insert(&mut self, index: usize, rect: Region) {
        let idx = index as u32;
        let (x0, x1) = axis_cells(rect.0[0], rect.0[2]);
        let (y0, y1) = axis_cells(rect.0[1], rect.0[3]);
        for cy in y0..=y1 {
            let row = cy * GRID_DIM;
            for cx in x0..=x1 {
                self.cells[row + cx].push(idx);
            }
        }
    }

    /// Finds the exact same obstacle index as [`Space::obstacle`]: the smallest
    /// index of any overlapping [`HARD`] obstacle, else the largest index of
    /// any overlapping soft obstacle.
    pub fn obstacle(&self, rect: Region, dynamic: &[Region], fixed: &[Region]) -> Option<usize> {
        let dyn_len = dynamic.len();
        let get = |idx: usize| -> Region {
            if idx < dyn_len {
                dynamic[idx]
            } else {
                fixed[idx - dyn_len]
            }
        };
        let (x0, x1) = axis_cells(rect.0[0], rect.0[2]);
        let (y0, y1) = axis_cells(rect.0[1], rect.0[3]);
        if x0 == x1 && y0 == y1 {
            let mut last = None;
            for &raw in &self.cells[y0 * GRID_DIM + x0] {
                let idx = raw as usize;
                let other = get(idx);
                if rect.overlaps(other) {
                    last = Some(idx);
                    if other.0[4] == HARD {
                        break;
                    }
                }
            }
            return last;
        }
        let mut best_hard: Option<usize> = None;
        let mut best_soft: Option<usize> = None;
        for cy in y0..=y1 {
            let row = cy * GRID_DIM;
            for cx in x0..=x1 {
                for &raw in &self.cells[row + cx] {
                    let idx = raw as usize;
                    if best_hard.is_some_and(|h| idx >= h) {
                        break;
                    }
                    let other = get(idx);
                    if other.0[4] == HARD {
                        if rect.overlaps(other) {
                            best_hard = Some(idx);
                            break;
                        }
                    } else if best_hard.is_none()
                        && best_soft.is_none_or(|s| idx > s)
                        && rect.overlaps(other)
                    {
                        best_soft = Some(idx);
                    }
                }
            }
        }
        best_hard.or(best_soft)
    }

    /// Returns whether `rect` overlaps any rectangle in `items`.
    pub fn any_overlap(&self, rect: Region, items: &[Region]) -> bool {
        let (x0, x1) = axis_cells(rect.0[0], rect.0[2]);
        let (y0, y1) = axis_cells(rect.0[1], rect.0[3]);
        for cy in y0..=y1 {
            let row = cy * GRID_DIM;
            for cx in x0..=x1 {
                for &raw in &self.cells[row + cx] {
                    if rect.overlaps(items[raw as usize]) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Returns the smallest index `i` such that `items[i].overlaps(rect)`.
    pub fn first_overlap(&self, rect: Region, items: &[Region]) -> Option<usize> {
        let (x0, x1) = axis_cells(rect.0[0], rect.0[2]);
        let (y0, y1) = axis_cells(rect.0[1], rect.0[3]);
        if x0 == x1 && y0 == y1 {
            for &raw in &self.cells[y0 * GRID_DIM + x0] {
                let idx = raw as usize;
                if items[idx].overlaps(rect) {
                    return Some(idx);
                }
            }
            return None;
        }
        let mut best: Option<usize> = None;
        for cy in y0..=y1 {
            let row = cy * GRID_DIM;
            for cx in x0..=x1 {
                for &raw in &self.cells[row + cx] {
                    let idx = raw as usize;
                    if best.is_some_and(|b| idx >= b) {
                        break;
                    }
                    if items[idx].overlaps(rect) {
                        best = Some(idx);
                        break;
                    }
                }
            }
        }
        best
    }
}

/// The obstacles a rectangle grows among, and the rectangles the graph
/// already holds.
pub(crate) struct Space<'a> {
    /// The obstacles of the objects on the lot, searched first.
    pub dynamic: &'a [Region],
    /// The room's own obstacles.
    pub fixed: &'a [Region],
    /// The graph's rectangles so far; a new one may not overlap them.
    pub regions: &'a [Region],
    /// The largest width or height a rectangle may grow to, or `None`.
    pub maximum: Option<i32>,
    /// Optional spatial index over `dynamic ++ fixed`.
    pub obstacle_grid: Option<&'a SpatialGrid>,
    /// Optional spatial index over `regions`.
    pub region_grid: Option<&'a SpatialGrid>,
}

impl Space<'_> {
    fn obstacles(&self) -> impl Iterator<Item = &Region> {
        self.dynamic.iter().chain(self.fixed)
    }

    /// The obstacle a rectangle lies in: the first hard one it overlaps,
    /// or else the last soft one, by index among the dynamic obstacles
    /// then the fixed ones.
    pub fn obstacle(&self, rect: Region) -> Option<usize> {
        if let Some(grid) = self.obstacle_grid {
            return grid.obstacle(rect, self.dynamic, self.fixed);
        }
        let mut last = None;
        for (i, &other) in self.obstacles().enumerate() {
            if rect.overlaps(other) {
                last = Some(i);
                if other.0[4] == HARD {
                    break;
                }
            }
        }
        last
    }

    /// The obstacle at an index [`Self::obstacle`] gave.
    #[inline]
    pub fn nth(&self, index: usize) -> Option<Region> {
        if index < self.dynamic.len() {
            self.dynamic.get(index).copied()
        } else {
            self.fixed.get(index - self.dynamic.len()).copied()
        }
    }

    fn occupied(&self, rect: Region) -> bool {
        if let Some(grid) = self.region_grid {
            return grid.any_overlap(rect, self.regions);
        }
        self.regions.iter().any(|&other| rect.overlaps(other))
    }

    fn too_large(&self, rect: Region) -> bool {
        self.maximum
            .is_some_and(|limit| rect.0[2] - rect.0[0] > limit || rect.0[3] - rect.0[1] > limit)
    }

    /// Grows a rectangle as far as it can stay in the same obstacle (or
    /// in none), clear of the graph's rectangles and within the maximum
    /// (0x469b26). Growth cycles left, top, right, bottom by a step of a
    /// quarter of the maximum, or 32; a step that fails is halved on the
    /// same side until a smaller one fits or it reaches 0. At most 200
    /// steps are tried, and the growth stops once all four sides are
    /// stuck. Returns whether the result has an area; a rectangle that
    /// starts too large, in a hard obstacle or on the graph is refused.
    pub fn expand(&self, rect: &mut Region) -> bool {
        let obstacle = self.obstacle(*rect);
        let containing = obstacle.and_then(|i| self.nth(i));
        if self.too_large(*rect)
            || containing.is_some_and(|r| r.0[4] == HARD)
            || self.occupied(*rect)
        {
            return false;
        }
        let step = self.maximum.map_or(32, |n| (n / 4).max(1));
        let mut steps = [-step, -step, step, step];
        let mut zeroes = 0;
        for iteration in 0..200 {
            let axis = iteration & 3;
            loop {
                let step = steps[axis];
                if step == 0 {
                    zeroes += 1;
                    break;
                }
                zeroes = 0;
                rect.0[axis] += step;
                let contained = containing.is_none_or(|r| {
                    r.0[0] <= rect.0[0]
                        && rect.0[2] <= r.0[2]
                        && r.0[1] <= rect.0[1]
                        && rect.0[3] <= r.0[3]
                });
                if !self.too_large(*rect)
                    && self.obstacle(*rect) == obstacle
                    && !self.occupied(*rect)
                    && contained
                {
                    break;
                }
                rect.0[axis] -= step;
                steps[axis] /= 2;
            }
            if zeroes >= 4 {
                break;
            }
        }
        rect.0[0] < rect.0[2] && rect.0[1] < rect.0[3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_grows_until_it_meets_a_wall() {
        let fixed = [Region([10, 0, 12, 100, HARD])];
        let space = Space {
            dynamic: &[],
            fixed: &fixed,
            regions: &[],
            maximum: Some(128),
            obstacle_grid: None,
            region_grid: None,
        };
        let mut rect = Region([2, 2, 3, 3, 0]);
        assert!(space.expand(&mut rect));
        // Left and top grow by 32, then halve back to the first step
        // that fits; the right side stops at the wall.
        assert_eq!(rect.0[2], 10);
        assert!(rect.0[0] < 2 && rect.0[1] < 2);
        assert!(rect.0[2] - rect.0[0] <= 128 && rect.0[3] - rect.0[1] <= 128);
    }

    #[test]
    fn a_seed_inside_a_hard_obstacle_is_refused() {
        let fixed = [Region([0, 0, 10, 10, HARD])];
        let space = Space {
            dynamic: &[],
            fixed: &fixed,
            regions: &[],
            maximum: None,
            obstacle_grid: None,
            region_grid: None,
        };
        let mut rect = Region([2, 2, 3, 3, 0]);
        assert!(!space.expand(&mut rect));
    }

    #[test]
    fn the_first_hard_overlap_wins_else_the_last_soft_one() {
        let dynamic = [Region([0, 0, 10, 10, 5]), Region([0, 0, 10, 10, 6])];
        let fixed = [Region([0, 0, 10, 10, HARD]), Region([0, 0, 10, 10, 7])];
        let grid = SpatialGrid::from_obstacles(&dynamic, &fixed);
        let space = Space {
            dynamic: &dynamic,
            fixed: &fixed,
            regions: &[],
            maximum: None,
            obstacle_grid: Some(&grid),
            region_grid: None,
        };
        assert_eq!(space.obstacle(Region([1, 1, 2, 2, 0])), Some(2));
        let soft_grid = SpatialGrid::from_obstacles(&dynamic, &[]);
        let soft = Space {
            dynamic: &dynamic,
            fixed: &[],
            regions: &[],
            maximum: None,
            obstacle_grid: Some(&soft_grid),
            region_grid: None,
        };
        assert_eq!(soft.obstacle(Region([1, 1, 2, 2, 0])), Some(1));
    }

    #[test]
    fn spatial_grid_matches_linear_scan_for_priority_and_edge_rects() {
        // Mix negative, cell-boundary, oversized, overlapping hard+soft rects.
        let dynamic = [
            Region([-20, -20, 70, 70, 5]),
            Region([60, 60, 200, 200, 10]),
            Region([128, 0, 192, 128, HARD]),
            Region([100, 100, 1050, 1050, 15]),
        ];
        let fixed = [
            Region([-7, -7, 7, 1030, HARD]),
            Region([0, 63, 130, 65, HARD]),
            Region([950, 950, 1200, 1200, 20]),
            Region([60, 60, 140, 140, 3]),
        ];
        let regions = [
            Region([0, 0, 64, 64, 0]),
            Region([64, 0, 128, 64, 0]),
            Region([30, 30, 90, 90, 0]),
            Region([960, 960, 1100, 1100, 0]),
        ];
        let obs_grid = SpatialGrid::from_obstacles(&dynamic, &fixed);
        let mut reg_grid = SpatialGrid::new();
        for (i, &r) in regions.iter().enumerate() {
            reg_grid.insert(i, r);
        }
        let linear = Space {
            dynamic: &dynamic,
            fixed: &fixed,
            regions: &regions,
            maximum: Some(128),
            obstacle_grid: None,
            region_grid: None,
        };
        let indexed = Space {
            dynamic: &dynamic,
            fixed: &fixed,
            regions: &regions,
            maximum: Some(128),
            obstacle_grid: Some(&obs_grid),
            region_grid: Some(&reg_grid),
        };
        let queries = [
            Region([-10, -10, 5, 5, 0]),
            Region([10, 10, 11, 11, 0]),
            Region([64, 64, 64, 64, 0]), // zero-width on exact cell boundary
            Region([63, 63, 65, 65, 0]), // crosses 4 cells
            Region([50, 50, 200, 200, 0]),
            Region([129, 10, 130, 11, 0]),
            Region([990, 990, 1150, 1150, 0]),
            Region([2000, 2000, 2001, 2001, 0]),
            Region([70, 70, 65, 65, 0]), // inverted
        ];
        for q in queries {
            assert_eq!(indexed.obstacle(q), linear.obstacle(q), "obstacle {q:?}");
            assert_eq!(indexed.occupied(q), linear.occupied(q), "occupied {q:?}");
            assert_eq!(
                reg_grid.first_overlap(q, &regions),
                regions.iter().position(|r| r.overlaps(q)),
                "first_overlap {q:?}"
            );
        }
    }
}
