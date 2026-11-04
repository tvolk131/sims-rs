//! Bounded, per-floor geometry in original 1/16-tile horizontal units.
use serde::{Deserialize, Serialize};
use sims_formats::execution::Slot;
use std::{cmp::Reverse, collections::BinaryHeap};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
impl Point {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    pub fn distance(self, other: Self) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}
impl Rect {
    pub fn overlaps(self, other: Self) -> bool {
        self.min.x < self.max.x
            && self.min.y < self.max.y
            && other.min.x < other.max.x
            && other.min.y < other.max.y
            && self.min.x < other.max.x
            && self.max.x > other.min.x
            && self.min.y < other.max.y
            && self.max.y > other.min.y
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Room {
    pub width: i32,
    pub height: i32,
    pub obstacles: Vec<Rect>,
}
impl Room {
    pub fn validate(&self) -> Result<(), String> {
        if !(16..=1024).contains(&self.width)
            || !(16..=1024).contains(&self.height)
            || self.obstacles.len() > 4096
        {
            return Err(
                "room must be 16..1024 subtiles per side, with at most 4096 obstacles".into(),
            );
        }
        for r in &self.obstacles {
            if r.min.x < 0
                || r.min.y < 0
                || r.max.x > self.width
                || r.max.y > self.height
                || r.min.x >= r.max.x
                || r.min.y >= r.max.y
            {
                return Err("invalid room obstacle".into());
            }
        }
        Ok(())
    }
    pub fn contains(&self, r: Rect) -> bool {
        r.min.x >= 0 && r.min.y >= 0 && r.max.x <= self.width && r.max.y <= self.height
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Body {
    /// Native one-based storey, separate from SLOT elevation within a storey.
    pub level: i32,
    pub position: Point,
    /// Sims compass bit: N=1, NE=2, E=4, SE=8, S=16, SW=32, W=64, NW=128.
    pub facing: u8,
    /// An explicit laboratory collision half-width, not an imported OBJD footprint.
    pub radius: i32,
    /// Original SLOT height units (the chair's seated height is 7).
    pub elevation: i32,
}
impl Body {
    pub fn bounds_at(&self, p: Point) -> Rect {
        Rect {
            min: Point::new(p.x - self.radius, p.y - self.radius),
            max: Point::new(p.x + self.radius, p.y + self.radius),
        }
    }
    pub fn validate(&self, room: &Room) -> Result<(), String> {
        if !(1..=2).contains(&self.level)
            || !self.facing.is_power_of_two()
            || !(0..=16).contains(&self.radius)
            || !(0..=15).contains(&self.elevation)
            || !(0..room.width).contains(&self.position.x)
            || !(0..room.height).contains(&self.position.y)
            || !room.contains(self.bounds_at(self.position))
        {
            return Err("invalid spatial body".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Destination {
    pub position: Point,
    pub facing: u8,
    pub entry_flags: i16,
}

fn rotate(p: Point, facing: u8) -> Result<Point, String> {
    if p.x == 0 && p.y == 0 && facing.is_power_of_two() {
        return Ok(p);
    }
    Ok(match facing {
        1 => p,
        4 => Point::new(-p.y, p.x),
        16 => Point::new(-p.x, -p.y),
        64 => Point::new(p.y, -p.x),
        _ => return Err("diagonal object orientation is outside the room model".into()),
    })
}
fn turn(direction: u8, rotation: u8) -> u8 {
    direction.rotate_left(rotation.trailing_zeros())
}
fn towards(from: Point, to: Point) -> Result<u8, String> {
    let (x, y) = (to.x - from.x, to.y - from.y);
    Ok(match (x.signum(), y.signum()) {
        (0, -1) => 1,
        (1, 0) => 4,
        (0, 1) => 16,
        (-1, 0) => 64,
        (0, 0) => 1,
        _ => return Err("non-cardinal facing is outside the room model".into()),
    })
}
pub(crate) fn offset(slot: &Slot, target: &Body) -> Result<Point, String> {
    if slot
        .offset
        .iter()
        .any(|v| !v.is_finite() || v.fract() != 0.0 || v.abs() > 256.0)
        || slot.offset[2] != 0.0
    {
        return Err("only integral horizontal SLOT offsets on one floor are supported".into());
    }
    let p = rotate(
        Point::new(slot.offset[0] as i32, slot.offset[1] as i32),
        target.facing,
    )?;
    Ok(Point::new(target.position.x + p.x, target.position.y + p.y))
}
/// A deliberately narrow, checked SLOT subset. Unsupported geometry is an error,
/// rather than a guessed destination that lets the behavior appear to succeed.
pub fn destinations(
    slot: &Slot,
    version: u32,
    target: &Body,
    caller: &Body,
) -> Result<Vec<Destination>, String> {
    if !(4..=10).contains(&version)
        || slot.kind != 3
        || slot.flags & !(0x55 | 512 | 2048 | 4096) != 0
        || !matches!(slot.height, 0 | 5)
        || slot.unknown != 0
        || !matches!(slot.max_size, 0 | 100)
        || slot.sitting != 0
        || slot.ground != 0
        || !slot.gradient.is_finite()
        || slot.facing == -3
    {
        return Err(
            "unsupported SLOT flags, height, posture, or metadata in the bounded room model".into(),
        );
    }
    let center = offset(slot, target)?;
    let on_point = slot.flags & 255 == 0 || slot.offset != [0.0; 3];
    let rotation = if slot.flags & 512 != 0 && !on_point {
        1
    } else {
        target.facing
    };
    let snap_dir = slot.flags & 4096 != 0;
    let mut points = Vec::new();
    if on_point || snap_dir {
        points.push((center, (slot.flags & 255) as u8));
    } else {
        if slot.standing <= 0 {
            return Err("routing to a non-standing destination is unsupported".into());
        }
        let scale = if version <= 9 { 16 } else { 1 };
        let [min, max, optimal] = slot
            .proximity
            .map(|n| n.checked_mul(scale).ok_or("SLOT proximity overflow"))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .unwrap();
        let max = if max == 0 { min } else { max };
        if min != max || !(1..=256).contains(&min) || (optimal != 0 && optimal != min) {
            return Err("only fixed-radius cardinal routing slots are supported".into());
        }
        for (flag, delta) in [
            (1, Point::new(0, -min)),
            (4, Point::new(min, 0)),
            (16, Point::new(0, min)),
            (64, Point::new(-min, 0)),
        ] {
            if slot.flags & flag != 0 {
                let d = rotate(delta, rotation)?;
                points.push((Point::new(center.x + d.x, center.y + d.y), flag as u8));
            }
        }
    }
    points
        .into_iter()
        .map(|(position, flags)| {
            let facing = if snap_dir {
                let direction = if slot.facing >= 0 {
                    match slot.facing {
                        0 | 2 | 4 | 6 => 1u8 << slot.facing,
                        _ => return Err("unsupported snap facing".into()),
                    }
                } else if flags == 0 {
                    1
                } else if flags.count_ones() == 1 {
                    flags
                } else {
                    return Err("ambiguous snap direction flags".into());
                };
                turn(direction, rotation)
            } else {
                match slot.facing {
                    -3 => caller.facing,
                    -2 => towards(position, target.position)?,
                    -1 => turn(towards(position, target.position)?, 16),
                    0 | 2 | 4 | 6 => turn(1u8 << slot.facing, target.facing),
                    _ => return Err("unsupported SLOT facing".into()),
                }
            };
            Ok(Destination {
                position,
                facing,
                entry_flags: i16::from(flags),
            })
        })
        .collect()
}

/// One subtile is 1000 distance units. sqrt(2) is rounded once, for deterministic timing.
pub const SUBTILE: u32 = 1000;
/// Straight rectangle-graph segment, retaining exact legacy unit-edge timing.
pub fn segment_distance(a: Point, b: Point) -> Option<u32> {
    if let Some(d) = step_distance(a, b) {
        return Some(d);
    }
    if a == b {
        return None;
    }
    let dx = f64::from(b.x - a.x);
    let dy = f64::from(b.y - a.y);
    Some(((dx * dx + dy * dy).sqrt() * f64::from(SUBTILE)).round() as u32)
}
/// Test a longer segment at every crossed subtile. Collision footprints remain
/// a world service; this samples the segment without changing its geometry.
pub(crate) fn segment_clear(a: Point, b: Point, mut clear: impl FnMut(Point) -> bool) -> bool {
    if step_distance(a, b).is_some() {
        return edge_clear(a, b, clear);
    }
    let count = (b.x - a.x).abs().max((b.y - a.y).abs());
    (1..=count).all(|i| {
        clear(Point::new(
            a.x + (b.x - a.x) * i / count,
            a.y + (b.y - a.y) * i / count,
        ))
    })
}
pub fn step_distance(a: Point, b: Point) -> Option<u32> {
    let dx = (i64::from(b.x) - i64::from(a.x)).abs();
    let dy = (i64::from(b.y) - i64::from(a.y)).abs();
    match (dx, dy) {
        (1, 0) | (0, 1) => Some(SUBTILE),
        (1, 1) => Some(1414),
        _ => None,
    }
}
pub fn step_facing(a: Point, b: Point) -> Option<u8> {
    step_distance(a, b)?;
    Some(match (b.x - a.x, b.y - a.y) {
        (0, -1) => 1,
        (1, -1) => 2,
        (1, 0) => 4,
        (1, 1) => 8,
        (0, 1) => 16,
        (-1, 1) => 32,
        (-1, 0) => 64,
        (-1, -1) => 128,
        _ => unreachable!(),
    })
}
/// Test the whole grid edge. Checking both orthogonal neighbors conservatively
/// excludes diagonal corner cutting for the caller's expanded collision bounds.
pub(crate) fn edge_clear(a: Point, b: Point, mut clear: impl FnMut(Point) -> bool) -> bool {
    step_distance(a, b).is_some()
        && clear(b)
        && (a.x == b.x
            || a.y == b.y
            || (clear(Point::new(a.x, b.y)) && clear(Point::new(b.x, a.y))))
}
/// Eight-direction Dijkstra search. Distance wins; equal distances prefer fewer
/// bends, then insertion order (N, NE, E, ...). Heading is part of the search state.
/// This is our deterministic policy, not a claim about the original route scorer.
pub(crate) fn pathfind(
    room: &Room,
    start: Point,
    goals: &[Destination],
    clear: impl FnMut(Point) -> bool,
) -> Option<(Destination, Vec<Point>)> {
    pathfind_scored(
        room,
        start,
        &goals.iter().map(|&g| (g, 0)).collect::<Vec<_>>(),
        clear,
    )
}
/// Travel length plus a destination penalty, preserving the native candidate
/// tradeoff without claiming the original rectangle-graph path search.
pub(crate) fn pathfind_scored(
    room: &Room,
    start: Point,
    goals: &[(Destination, u32)],
    mut clear: impl FnMut(Point) -> bool,
) -> Option<(Destination, Vec<Point>)> {
    let mut best: Option<(u32, u32, usize, usize)> = None;
    // World geometry cannot change during one synchronous route plan. Cache
    // occupancy across the eight heading states and diagonal corner checks.
    let mut occupancy = vec![None; (room.width * room.height) as usize];
    let mut clear = |p: Point| {
        if p.x < 0 || p.y < 0 || p.x >= room.width || p.y >= room.height {
            return false;
        }
        let i = (p.y * room.width + p.x) as usize;
        if let Some(v) = occupancy[i] {
            v
        } else {
            let v = clear(p);
            occupancy[i] = Some(v);
            v
        }
    };

    let stride = room.width as usize;
    let index = |p: Point, d: usize| (p.y as usize * stride + p.x as usize) * 8 + d;
    let point = |i: usize| Point::new(((i / 8) % stride) as i32, ((i / 8) / stride) as i32);
    let size = (room.width * room.height) as usize * 8;
    let mut previous = vec![usize::MAX; size];
    let mut costs = vec![(u32::MAX, u32::MAX); size];
    let mut queue = BinaryHeap::new();
    let mut order = 0u32;
    // All initial headings are free: the locomotion timeline performs the initial turn.
    for d in 0..8 {
        let i = index(start, d);
        costs[i] = (0, 0);
        previous[i] = i;
        queue.push(Reverse((0u32, 0u32, order, i)));
        order += 1;
    }
    while let Some(Reverse((cost, bends, _, i))) = queue.pop() {
        if costs[i] != (cost, bends) {
            continue;
        }
        let p = point(i);
        if best.is_some_and(|b| cost > b.0) {
            break;
        }
        for (goal, (_g, penalty)) in goals
            .iter()
            .enumerate()
            .filter(|(_, (g, _))| g.position == p)
        {
            let candidate = (cost.saturating_add(*penalty), bends, goal, i);
            if best.is_none_or(|b| candidate < b) {
                best = Some(candidate);
            }
        }
        for (d, (dx, dy)) in [
            (0, -1),
            (1, -1),
            (1, 0),
            (1, 1),
            (0, 1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
        ]
        .into_iter()
        .enumerate()
        {
            let q = Point::new(p.x + dx, p.y + dy);
            if q.x < 0 || q.y < 0 || q.x >= room.width || q.y >= room.height {
                continue;
            }
            let j = index(q, d);
            let score = (
                cost + step_distance(p, q).unwrap(),
                bends + u32::from(d != i % 8),
            );
            if score < costs[j] && edge_clear(p, q, &mut clear) {
                costs[j] = score;
                previous[j] = i;
                queue.push(Reverse((score.0, score.1, order, j)));
                order += 1;
            }
        }
    }
    let (_, _, goal, mut j) = best?;
    let mut path = vec![point(j)];
    while previous[j] != j {
        j = previous[j];
        path.push(point(j));
    }
    path.reverse();
    Some((goals[goal].0, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn room() -> Room {
        Room {
            width: 32,
            height: 32,
            obstacles: vec![],
        }
    }
    #[test]
    fn diagonal_shortest_path_has_weighted_distance_and_no_staircase_bends() {
        let (goal, path) = pathfind(
            &room(),
            Point::new(4, 20),
            &[Destination {
                position: Point::new(16, 8),
                facing: 2,
                entry_flags: 0,
            }],
            |_| true,
        )
        .unwrap();
        assert_eq!(path.len(), 13);
        assert_eq!(path.last(), Some(&goal.position));
        assert!(path.windows(2).all(|w| step_facing(w[0], w[1]) == Some(2)));
        assert_eq!(
            path.windows(2)
                .map(|w| step_distance(w[0], w[1]).unwrap())
                .sum::<u32>(),
            12 * 1414
        );
    }
    #[test]
    fn diagonal_cannot_squeeze_between_touching_corners_or_a_dynamic_side_blocker() {
        let start = Point::new(10, 10);
        let end = Point::new(11, 11);
        let clear = |p: Point| p == start || p == end;
        assert!(!edge_clear(start, end, clear));
        assert!(
            pathfind(
                &room(),
                start,
                &[Destination {
                    position: end,
                    facing: 8,
                    entry_flags: 0
                }],
                clear
            )
            .is_none()
        );
        assert!(!edge_clear(start, end, |p| p != Point::new(10, 11)));
        assert!(!edge_clear(start, end, |p| p != Point::new(11, 10)));
        assert!(edge_clear(start, end, |_| true));
    }
}
