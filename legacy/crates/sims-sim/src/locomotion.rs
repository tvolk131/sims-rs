//! Serializable, integer-clock locomotion. Rendering samples this timeline; it cannot advance it.
//! Distances are 1/1000 subtiles. Float conversion is confined to presentation sampling.
use crate::geometry::{Point, SUBTILE, segment_distance, step_distance, step_facing};
use serde::{Deserialize, Serialize};
use sims_vm::Program;

pub const WALK: &str = "a2o-walking-loop";
pub const STAND: &str = "a2o-stand";
const ANGLE: i32 = 1024;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Start,
    Walk,
    Stop,
    Turn,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Phase {
    pub kind: Kind,
    pub clip: String,
    pub start_tick: u32,
    pub ticks: u32,
    pub from_distance: u32,
    pub to_distance: u32,
    /// Unwrapped clockwise eighth-turns / 1024 from north.
    pub from_heading: i32,
    pub to_heading: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Locomotion {
    pub initial_facing: u8,
    pub distances: Vec<u32>,
    pub phases: Vec<Phase>,
    pub elapsed: u32,
}
/// Presentation sample. Heading is clockwise radians from north; position is subtiles.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub position: [f32; 2],
    pub heading: f32,
    pub distance: f32,
    pub phase: usize,
    pub progress: f32,
}
fn delta(from: i32, to: i32) -> i32 {
    (to - from + 4 * ANGLE).rem_euclid(8 * ANGLE) - 4 * ANGLE
}
fn span(program: &Program, clip: &str, fallback: u32) -> u32 {
    // Optional for asset-free headless fixtures. Loaded source distances are fingerprinted.
    program
        .animations
        .get(clip)
        .filter(|a| a.clip.distance.is_finite() && a.clip.distance > 0.)
        .map_or(fallback, |a| {
            (a.clip.distance * (16_000. / 3.))
                .round()
                .clamp(1000., 64_000.) as u32
        })
}
fn foot(program: &Program, distance: u32) -> &'static str {
    let cycle = span(program, WALK, 21_706);
    if distance % cycle < cycle / 2 {
        "left"
    } else {
        "right"
    }
}
fn turn_clip(from: i32, to: i32, walking: bool, foot: &str) -> String {
    let change = to - from;
    let angle = if change.abs() <= ANGLE {
        45
    } else if change.abs() <= 2 * ANGLE {
        90
    } else {
        180
    };
    let side = if change > 0 { "cw" } else { "ccw" };
    if walking && angle <= 90 {
        format!("a2o-walking-turn-{angle}-{side}-{foot}")
    } else {
        format!("a2o-standing-turn-{angle}-{side}")
    }
}
impl Locomotion {
    pub fn plan(
        program: &Program,
        path: &[Point],
        initial_facing: u8,
        final_facing: u8,
    ) -> Result<Self, String> {
        if path.is_empty()
            || path.len() > 65536
            || !initial_facing.is_power_of_two()
            || !final_facing.is_power_of_two()
        {
            return Err("invalid locomotion path or facing".into());
        }
        let mut distances = vec![0];
        let mut directions = Vec::new();
        for w in path.windows(2) {
            distances.push(
                distances.last().unwrap()
                    + segment_distance(w[0], w[1]).ok_or("zero-length locomotion segment")?,
            );
            directions.push(if let Some(facing) = step_facing(w[0], w[1]) {
                facing.trailing_zeros() as i32 * ANGLE
            } else {
                (f64::from(w[1].x - w[0].x).atan2(f64::from(w[0].y - w[1].y))
                    / std::f64::consts::FRAC_PI_4
                    * f64::from(ANGLE))
                .round() as i32
            });
        }
        let mut plan = Self {
            initial_facing,
            distances,
            phases: vec![],
            elapsed: 0,
        };
        let mut heading = initial_facing.trailing_zeros() as i32 * ANGLE;
        if let Some(&first) = directions.first() {
            let next = heading + delta(heading, first);
            plan.standing_turn(0, heading, next);
            heading = next;
            // Consolidated straight runs keep turns on actual bends, not grid steps.
            let mut corners = vec![0usize];
            for i in 1..directions.len() {
                if directions[i] != directions[i - 1] {
                    corners.push(i);
                }
            }
            corners.push(path.len() - 1);
            let total = *plan.distances.last().unwrap();
            let first_run = plan.distances[corners[1]];
            let start = span(program, "a2o-walking-start", 8648).min(first_run / 3);
            plan.push(
                Kind::Start,
                "a2o-walking-start".into(),
                0,
                start,
                heading,
                heading,
                (2 * start).div_ceil(SUBTILE).max(1),
            );
            let last_run = total - plan.distances[corners[corners.len() - 2]];
            let stop_clip = format!("a2o-walking-stop-{}", foot(program, total));
            let stop = span(program, &stop_clip, 8127).min(last_run / 3);
            let mut distance = start;
            for c in 1..corners.len() - 1 {
                let i = corners[c];
                let at = plan.distances[i];
                let next = heading + delta(heading, directions[i]);
                if (next - heading).abs() > 2 * ANGLE {
                    // A tight reversal uses a planted turn, rather than sliding a standing clip.
                    plan.walk(distance, at, heading);
                    plan.standing_turn(at, heading, next);
                    distance = at;
                    heading = next;
                    continue;
                }
                let clip = turn_clip(heading, next, true, foot(program, at));
                let half = (span(program, &clip, 16000) / 2)
                    .min((at - plan.distances[corners[c - 1]]) / 3)
                    .min((plan.distances[corners[c + 1]] - at) / 3);
                let a = at - half;
                let b = at + half;
                plan.walk(distance, a, heading);
                plan.push(
                    Kind::Turn,
                    clip,
                    a,
                    b,
                    heading,
                    next,
                    (b - a).div_ceil(SUBTILE).max(4),
                );
                distance = b;
                heading = next;
            }
            plan.walk(distance, total - stop, heading);
            plan.push(
                Kind::Stop,
                stop_clip,
                total - stop,
                total,
                heading,
                heading,
                (2 * stop).div_ceil(SUBTILE).max(1),
            );
        }
        let end = heading + delta(heading, final_facing.trailing_zeros() as i32 * ANGLE);
        plan.standing_turn(*plan.distances.last().unwrap(), heading, end);
        Ok(plan)
    }
    #[allow(clippy::too_many_arguments)]
    fn push(&mut self, kind: Kind, clip: String, a: u32, b: u32, from: i32, to: i32, ticks: u32) {
        let start_tick = self.duration();
        self.phases.push(Phase {
            kind,
            clip,
            start_tick,
            ticks,
            from_distance: a,
            to_distance: b,
            from_heading: from,
            to_heading: to,
        });
    }
    fn walk(&mut self, a: u32, b: u32, heading: i32) {
        if b > a {
            self.push(
                Kind::Walk,
                WALK.into(),
                a,
                b,
                heading,
                heading,
                (b - a).div_ceil(SUBTILE),
            );
        }
    }
    fn standing_turn(&mut self, distance: u32, from: i32, to: i32) {
        if from != to {
            self.push(
                Kind::Turn,
                turn_clip(from, to, false, "left"),
                distance,
                distance,
                from,
                to,
                4 + (6 * (to - from).unsigned_abs()).div_ceil(ANGLE as u32),
            );
        }
    }
    pub fn duration(&self) -> u32 {
        self.phases.last().map_or(0, |p| p.start_tick + p.ticks)
    }
    pub fn distance(&self) -> u32 {
        let Some(p) = self
            .phases
            .iter()
            .find(|p| self.elapsed < p.start_tick + p.ticks)
        else {
            return *self.distances.last().unwrap();
        };
        let t = u64::from(self.elapsed - p.start_tick);
        let n = u64::from(p.ticks);
        let amount = match p.kind {
            Kind::Start => t * t,
            Kind::Stop => 2 * t * n - t * t,
            _ => t * n,
        };
        p.from_distance + ((u64::from(p.to_distance - p.from_distance) * amount) / (n * n)) as u32
    }
    pub fn next(&self) -> usize {
        self.distances
            .partition_point(|d| *d <= self.distance())
            .max(1)
    }
    /// Integer simulation anchor along a long native segment. Unit grid paths
    /// retain their previous anchor semantics, independent of rendered samples.
    pub fn anchor(&self, path: &[Point]) -> Point {
        let next = self.next();
        let a = path[next - 1];
        let Some(&b) = path.get(next) else {
            return a;
        };
        if step_distance(a, b).is_some() {
            return a;
        }
        let d = i64::from(self.distance() - self.distances[next - 1]);
        let length = i64::from(self.distances[next] - self.distances[next - 1]);
        Point::new(
            a.x + (i64::from(b.x - a.x) * d / length) as i32,
            a.y + (i64::from(b.y - a.y) * d / length) as i32,
        )
    }
    pub fn facing(&self) -> u8 {
        let Some(p) = self
            .phases
            .iter()
            .find(|p| self.elapsed < p.start_tick + p.ticks)
        else {
            return self.phases.last().map_or(self.initial_facing, |p| {
                1 << ((p.to_heading + ANGLE / 2).div_euclid(ANGLE)).rem_euclid(8)
            });
        };
        // Quantized object-data direction; the exact turn remains in this saved timeline.
        let t = i64::from(self.elapsed - p.start_tick);
        let n = i64::from(p.ticks);
        let smooth = t * t * (3 * n - 2 * t);
        let heading = i64::from(p.from_heading) * n * n * n
            + i64::from(p.to_heading - p.from_heading) * smooth;
        let scale = n * n * n * i64::from(ANGLE);
        let eighth = (heading + scale / 2).div_euclid(scale);
        1 << eighth.rem_euclid(8)
    }
    pub fn sample(&self, path: &[Point], fraction: f32) -> Sample {
        let time = (self.elapsed as f32 + fraction.clamp(0., 1.)).min(self.duration() as f32);
        let phase = self
            .phases
            .partition_point(|p| time >= (p.start_tick + p.ticks) as f32)
            .min(self.phases.len().saturating_sub(1));
        let (distance, heading, progress) = if let Some(p) = self.phases.get(phase) {
            let t = ((time - p.start_tick as f32) / p.ticks as f32).clamp(0., 1.);
            let f = match p.kind {
                Kind::Start => t * t,
                Kind::Stop => t * (2. - t),
                _ => t,
            };
            let h = t * t * (3. - 2. * t);
            (
                p.from_distance as f32 + (p.to_distance - p.from_distance) as f32 * f,
                (p.from_heading as f32 + (p.to_heading - p.from_heading) as f32 * h)
                    * (std::f32::consts::FRAC_PI_4 / ANGLE as f32),
                t,
            )
        } else {
            (
                0.,
                self.initial_facing.trailing_zeros() as f32 * std::f32::consts::FRAC_PI_4,
                0.,
            )
        };
        let next = self
            .distances
            .partition_point(|d| *d as f32 <= distance)
            .max(1);
        let p = path[next - 1];
        let position = if let Some(q) = path.get(next) {
            let f = (distance - self.distances[next - 1] as f32)
                / (self.distances[next] - self.distances[next - 1]) as f32;
            [
                p.x as f32 + (q.x - p.x) as f32 * f,
                p.y as f32 + (q.y - p.y) as f32 * f,
            ]
        } else {
            [p.x as f32, p.y as f32]
        };
        Sample {
            position,
            heading,
            distance,
            phase,
            progress,
        }
    }
}
