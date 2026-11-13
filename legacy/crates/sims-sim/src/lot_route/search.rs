//! Original 4823fa goal construction and retained-room search policy. World
//! enumeration, callbacks and rectangle production belong to the outer caller.
use crate::{Point, region_path, regions::Region, routing::CandidateSet};
use serde::{Deserialize, Serialize};

/// Values captured at wrapper entry, before object-function callbacks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Actor {
    pub special: bool,
    pub ground: bool,
    pub level: i32,
    pub outdoors: bool,
}
impl Actor {
    pub fn configure(self, half_tiles: i32, objects: i32, positive_goals: i32) -> Config {
        Config::prepare(Input {
            half_tiles,
            objects,
            special: self.special,
            ground: self.ground,
            level: self.level,
            outdoors: self.outdoors,
            positive_goals,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Input {
    /// Retained room+38/+40, not architectural measurements recomputed now.
    pub half_tiles: i32,
    pub objects: i32,
    pub special: bool,
    pub ground: bool,
    pub level: i32,
    pub outdoors: bool,
    pub positive_goals: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub bounded: bool,
    /// The unbounded branch retains 64 in this otherwise unused field.
    pub maximum: i32,
    pub free_cost: i32,
    pub limit: i32,
    pub preliminary: bool,
}
impl Config {
    pub fn prepare(input: Input) -> Self {
        let area = input.half_tiles / 2;
        // Unordered x87 comparisons do not take JA. Rust's > has the same
        // behavior for 0/0. For signed i32 ratios, rounding at binary64 cannot
        // cross the native binary64 0.2 threshold: neighboring ratios differ
        // by at least 1/(5*2^31); the exact 1/5 case also does not take JA.
        let bounded = input.special || {
            let dense = f64::from(input.objects) / f64::from(input.half_tiles) > 0.2;
            !dense && area >= 100
        };
        Self {
            bounded,
            maximum: if bounded { 128 } else { 64 },
            free_cost: if input.ground && input.outdoors && input.level == 1 {
                3
            } else {
                0
            },
            limit: area
                .wrapping_mul(input.positive_goals)
                .wrapping_add(50)
                .min(1000),
            preliminary: bounded && area > 256,
        }
    }
    /// Both original loops test iterations <= signed limit before each step.
    pub fn steps(self) -> usize {
        if self.limit < 0 {
            0
        } else {
            self.limit as usize + 1
        }
    }
    /// 482abc rejects an empty vector. A one-point path is nonempty too.
    pub fn preliminary_result(self, points: usize) -> Option<u8> {
        (self.preliminary && points == 0).then_some(3)
    }
    pub fn run(
        self,
        start: Point,
        goals: &[Region],
        dynamic: &[Region],
        fixed: &[Region],
    ) -> Result<Outcome, String> {
        let pass = |bounded: bool| {
            region_path::search(region_path::Query {
                start,
                goals,
                dynamic,
                fixed,
                maximum: bounded.then_some(self.maximum),
                aspect: None,
                free_cost: self.free_cost,
                smooth: true,
                steps: Some(self.steps()),
            })
        };
        let preliminary = self.preliminary.then(|| pass(false)).transpose()?;
        if preliminary
            .as_ref()
            .is_some_and(|r| self.preliminary_result(r.points.len()).is_some())
        {
            return Ok(Outcome {
                preliminary,
                search: None,
                failure_result: Some(3),
            });
        }
        // 482f3e resets the first pass before initializing the second. Each
        // search owns fresh graph vectors; no preliminary regions leak through.
        let search = pass(self.bounded)?;
        //482e70 publishes3 for an empty final path as well as an empty
        //preliminary path. The selected goal is not the caller's result byte.
        let failure_result = search.points.is_empty().then_some(3);
        Ok(Outcome {
            preliminary,
            search: Some(search),
            failure_result,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub preliminary: Option<region_path::Result>,
    pub search: Option<region_path::Result>,
    pub failure_result: Option<u8>,
}

/// 482861 preserves positive candidate order and its original index mapping.
/// Coordinates and score arithmetic wrap as the original 32-bit instructions.
pub fn goals(set: &CandidateSet) -> (Vec<Region>, Vec<usize>) {
    set.candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.score > 0)
        .map(|(index, c)| {
            let [x, y] = c.position;
            (
                Region([
                    y,
                    x,
                    y.wrapping_add(1),
                    x.wrapping_add(1),
                    set.maximum.wrapping_sub(c.score).wrapping_mul(16),
                ]),
                index,
            )
        })
        .unzip()
}
