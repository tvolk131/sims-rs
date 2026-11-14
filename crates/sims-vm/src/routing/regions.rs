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
}

impl Space<'_> {
    fn obstacles(&self) -> impl Iterator<Item = &Region> {
        self.dynamic.iter().chain(self.fixed)
    }

    /// The obstacle a rectangle lies in: the first hard one it overlaps,
    /// or else the last soft one, by index among the dynamic obstacles
    /// then the fixed ones.
    pub fn obstacle(&self, rect: Region) -> Option<usize> {
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
    pub fn nth(&self, index: usize) -> Option<Region> {
        self.obstacles().nth(index).copied()
    }

    fn occupied(&self, rect: Region) -> bool {
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
        };
        let mut rect = Region([2, 2, 3, 3, 0]);
        assert!(!space.expand(&mut rect));
    }

    #[test]
    fn the_first_hard_overlap_wins_else_the_last_soft_one() {
        let dynamic = [Region([0, 0, 10, 10, 5]), Region([0, 0, 10, 10, 6])];
        let fixed = [Region([0, 0, 10, 10, HARD]), Region([0, 0, 10, 10, 7])];
        let space = Space {
            dynamic: &dynamic,
            fixed: &fixed,
            regions: &[],
            maximum: None,
        };
        assert_eq!(space.obstacle(Region([1, 1, 2, 2, 0])), Some(2));
        let soft = Space {
            dynamic: &dynamic,
            fixed: &[],
            regions: &[],
            maximum: None,
        };
        assert_eq!(soft.obstacle(Region([1, 1, 2, 2, 0])), Some(1));
    }
}
