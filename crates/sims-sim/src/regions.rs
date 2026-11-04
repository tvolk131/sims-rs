//! Native rectangular free-space expansion, the building block of the original
//! within-room graph. Geometry is supplied in the original axis order.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Region(pub [i32; 5]);
impl Region {
    pub(crate) fn overlaps(self, other: Self) -> bool {
        let a = self.0;
        let b = other.0;
        a[1] < b[3] && b[1] < a[3] && a[0] < b[2] && b[0] < a[2]
    }
}
pub struct Space<'a> {
    pub dynamic: &'a [Region],
    pub fixed: &'a [Region],
    pub regions: &'a [Region],
    pub maximum: Option<i32>,
    pub aspect: Option<i32>,
}
impl Space<'_> {
    fn obstacles(&self) -> impl Iterator<Item = &Region> {
        self.dynamic.iter().chain(self.fixed)
    }
    /// The first hard overlap wins; otherwise the *last* soft overlap wins.
    /// Identity matters when expanding, even if two soft costs are equal.
    pub(crate) fn obstacle(&self, rect: Region) -> Option<usize> {
        let mut last = None;
        for (i, &other) in self.obstacles().enumerate() {
            if rect.overlaps(other) {
                last = Some(i);
                if other.0[4] == i32::MAX {
                    break;
                }
            }
        }
        last
    }
    fn occupied(&self, rect: Region) -> bool {
        self.regions.iter().any(|&other| rect.overlaps(other))
    }
    fn too_large(&self, rect: Region) -> bool {
        self.maximum
            .is_some_and(|limit| rect.0[2] - rect.0[0] > limit || rect.0[3] - rect.0[1] > limit)
    }
    /// Original 0x469b26. Successful growth cycles left, top, right, bottom;
    /// failed steps halve on the same axis until a smaller step fits.
    pub fn expand(&self, rect: &mut Region) -> bool {
        let initial = *rect;
        let obstacle = self.obstacle(*rect);
        let containing = obstacle.and_then(|i| self.obstacles().nth(i)).copied();
        if self.too_large(*rect)
            || containing.is_some_and(|r| r.0[4] == i32::MAX)
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
        if let Some(ratio) = self.aspect {
            let width = rect.0[2] - rect.0[0];
            let height = rect.0[3] - rect.0[1];
            if height < width {
                if (ratio as f32) < width as f32 / height as f32 {
                    let size = ratio * height;
                    let start = ((initial.0[0] + initial.0[2]) / 2 - size / 2)
                        .clamp(rect.0[0], rect.0[2] - size);
                    rect.0[0] = start;
                    rect.0[2] = start + size;
                }
            } else if (ratio as f32) < height as f32 / width as f32 {
                let size = ratio * width;
                let start = ((initial.0[1] + initial.0[3]) / 2 - size / 2)
                    .clamp(rect.0[1], rect.0[3] - size);
                rect.0[1] = start;
                rect.0[3] = start + size;
            }
        }
        rect.0[0] < rect.0[2] && rect.0[1] < rect.0[3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Deserialize)]
    struct Input {
        seed: Region,
        dynamic: Vec<Region>,
        #[serde(rename = "static")]
        fixed: Vec<Region>,
        regions: Vec<Region>,
        bounded: bool,
        maximum: i32,
        aspect: bool,
        ratio: i32,
    }
    #[derive(Deserialize)]
    struct Output {
        accepted: bool,
        rectangle: Region,
    }
    #[derive(Deserialize)]
    struct Case {
        input: Input,
        original: Output,
    }
    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn matches_original_free_space_expansion() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../tests/fixtures/route-regions.json")).unwrap();
        for (i, case) in fixture.cases.into_iter().enumerate() {
            let c = case.input;
            let space = Space {
                dynamic: &c.dynamic,
                fixed: &c.fixed,
                regions: &c.regions,
                maximum: c.bounded.then_some(c.maximum),
                aspect: c.aspect.then_some(c.ratio),
            };
            let mut rect = c.seed;
            assert_eq!(space.expand(&mut rect), case.original.accepted, "case {i}");
            assert_eq!(rect, case.original.rectangle, "case {i}");
        }
    }
}
