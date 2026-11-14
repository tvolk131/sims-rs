//! The planner's search: a graph of free rectangles labelled by goal,
//! searched with A*, then the path's points slid along the edges between
//! rectangles to shorten it (0x46a8e1 starts a search, 0x482ec5 takes a
//! step; the graph's routines are at 0x469b26 to 0x46a8e1).
//!
//! Ported from the old prototype (legacy/crates/sims-sim/src/region_path.rs:
//! `Graph::init`, `Graph::neighbors`, `Graph::transition`, `search`),
//! which compared the search with the original's code run alone. The
//! arithmetic here is the FPU's under the game's control word: every
//! addition, product, quotient and square root rounds to single
//! precision.

use super::regions::{HARD, Region, Space};
use crate::fault::Trap;
use sims_core::{Ext, F32};

/// A point: the first coordinate (object +0xf8) and the second (+0xf4).
pub(crate) type Point = [i32; 2];

#[derive(Debug, Clone)]
struct Node {
    goal: Option<usize>,
    region: Option<usize>,
    neighbors: Option<Vec<usize>>,
    cost: i32,
    point: Point,
    parent: Option<usize>,
    f: F32,
    g: F32,
}

impl Node {
    fn new(goal: Option<usize>, region: Option<usize>, cost: i32) -> Self {
        Self {
            goal,
            region,
            cost,
            neighbors: None,
            point: [0, 0],
            parent: None,
            f: F32::ZERO,
            g: F32::ZERO,
        }
    }
}

/// What a search is asked.
pub(crate) struct Query<'a> {
    pub start: Point,
    pub goals: &'a [Region],
    pub dynamic: &'a [Region],
    pub fixed: &'a [Region],
    pub maximum: Option<i32>,
    /// The cost given to a free rectangle found while searching
    /// (search +0x2c). The start's and the goals' rectangles keep theirs.
    pub free_cost: i32,
    /// How many steps the search may take, path smoothing included.
    pub steps: usize,
}

/// What a search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    /// The goal reached, or `None`.
    pub goal: Option<usize>,
    /// The path, from the start to the centre of the goal.
    pub points: Vec<Point>,
    /// How many steps the search took.
    pub iterations: usize,
}

struct Graph<'a> {
    query: Query<'a>,
    regions: Vec<Region>,
    nodes: Vec<Node>,
    goal_nodes: Vec<usize>,
    start_nodes: Vec<usize>,
}

fn center(r: Region) -> Point {
    [(r.0[0] + r.0[2]) / 2, (r.0[1] + r.0[3]) / 2]
}

/// The distance between two points, in single precision.
fn length(a: Point, b: Point) -> Ext {
    let x = Ext::from_i32(a[0].wrapping_sub(b[0]));
    let y = Ext::from_i32(a[1].wrapping_sub(b[1]));
    ((x * x).single() + (y * y).single())
        .single()
        .sqrt()
        .single()
}

/// The rectangle where two rectangles touch, one wider on every side.
fn edge(a: Region, b: Region) -> Region {
    Region([
        a.0[0].max(b.0[0]) - 1,
        a.0[1].max(b.0[1]) - 1,
        a.0[2].min(b.0[2]) + 1,
        a.0[3].min(b.0[3]) + 1,
        0,
    ])
}

fn limit(what: &str) -> Trap {
    Trap::Unsupported(format!(
        "the route search passed its safety limit on {what}; the original has none"
    ))
}

impl Graph<'_> {
    fn space(&self) -> Space<'_> {
        Space {
            dynamic: self.query.dynamic,
            fixed: self.query.fixed,
            regions: &self.regions,
            maximum: self.query.maximum,
        }
    }

    fn obstacle(&self, rect: Region) -> Option<Region> {
        let space = self.space();
        space.obstacle(rect).and_then(|i| space.nth(i))
    }

    fn region_at(&self, rect: Region) -> Option<usize> {
        self.regions.iter().position(|r| r.overlaps(rect))
    }

    /// The start's rectangle and node 0, the end node 1, and for each goal
    /// not inside a hard obstacle a node in its rectangle and a node in
    /// the start's.
    fn init(&mut self) {
        let start = self.query.start;
        let mut region = Region([start[0], start[1], start[0] + 1, start[1] + 1, 0]);
        self.space().expand(&mut region);
        self.regions.push(region);
        self.nodes.push(Node::new(None, Some(0), 0));
        self.nodes[0].point = start;
        self.nodes.push(Node::new(None, None, 0));
        for (i, &goal) in self.query.goals.iter().enumerate() {
            let obstacle = self.obstacle(goal);
            if obstacle.is_some_and(|r| r.0[4] == HARD) {
                continue;
            }
            let region = if let Some(id) = self.region_at(goal) {
                id
            } else {
                let mut r = goal;
                r.0[4] = obstacle.map_or(0, |r| r.0[4]);
                if !self.space().expand(&mut r) {
                    continue;
                }
                self.regions.push(r);
                self.regions.len() - 1
            };
            self.goal_nodes.push(self.nodes.len());
            self.nodes
                .push(Node::new(Some(i), Some(region), self.regions[region].0[4]));
            self.start_nodes.push(self.nodes.len());
            self.nodes.push(Node::new(Some(i), Some(0), goal.0[4]));
        }
    }

    /// A node's neighbours, found once: the rectangles along each side of
    /// its rectangle, walked clockwise from the top-left corner, each
    /// grown when first seen.
    fn neighbors(&mut self, id: usize) -> Result<Vec<usize>, Trap> {
        if let Some(n) = &self.nodes[id].neighbors {
            return Ok(n.clone());
        }
        let mut neighbors = vec![];
        if id == 0 {
            neighbors = self.start_nodes.clone();
        } else if id != 1 {
            let region = self.regions[self.nodes[id].region.ok_or_else(|| limit("a node"))?];
            let [x0, y0, x1, y1, _] = region.0;
            let mut seed = Region([x0, y0 - 1, x0 + 1, y0, 0]);
            let mut side = 0;
            let mut steps = 0;
            while side < 4 {
                steps += 1;
                if steps > 8192 || self.nodes.len() > 65536 {
                    return Err(limit("a rectangle's neighbours"));
                }
                let obstacle = self.obstacle(seed);
                let bounds = if let Some(hard) = obstacle.filter(|r| r.0[4] == HARD) {
                    hard
                } else {
                    let region = if let Some(i) = self.region_at(seed) {
                        i
                    } else {
                        let mut r = seed;
                        if !self.space().expand(&mut r) {
                            return Err(Trap::Unsupported(
                                "the route search could not grow a free seed beside a \
                                 rectangle (0x469eba); the original's next step is not ported"
                                    .into(),
                            ));
                        }
                        r.0[4] = obstacle.map_or(self.query.free_cost, |r| r.0[4]);
                        self.regions.push(r);
                        self.regions.len() - 1
                    };
                    let goal = self.nodes[id].goal;
                    let node = self
                        .nodes
                        .iter()
                        .position(|n| n.goal == goal && n.region == Some(region))
                        .unwrap_or_else(|| {
                            self.nodes.push(Node::new(
                                goal,
                                Some(region),
                                self.regions[region].0[4],
                            ));
                            self.nodes.len() - 1
                        });
                    neighbors.push(node);
                    self.regions[region]
                };
                let [a, b, c, d, _] = bounds.0;
                let (dx, dy, turn) = match side {
                    0 if seed.0[2] <= x1 && c < x1 => (c - seed.0[0], 0, false),
                    0 => (x1 - seed.0[0], 1, true),
                    1 if seed.0[3] <= y1 && d < y1 => (0, d - seed.0[1], false),
                    1 => (-1, y1 - seed.0[1], true),
                    2 if seed.0[0] >= x0 && a > x0 => (a - seed.0[0] - 1, 0, false),
                    2 => (x0 - seed.0[0] - 1, -1, true),
                    3 if seed.0[1] >= y0 && b > y0 => (0, b - seed.0[1] - 1, false),
                    _ => (1, y0 - seed.0[1] - 1, true),
                };
                seed.0[0] += dx;
                seed.0[2] += dx;
                seed.0[1] += dy;
                seed.0[3] += dy;
                side += usize::from(turn);
            }
            if self.goal_nodes.contains(&id) {
                neighbors.push(1);
            }
            if self.start_nodes.contains(&id) {
                neighbors.extend(self.goal_nodes.iter().copied().filter(|&n| {
                    self.nodes[n].goal == self.nodes[id].goal
                        && self.nodes[n].region == self.nodes[id].region
                }));
            }
        }
        self.nodes[id].neighbors = Some(neighbors.clone());
        Ok(neighbors)
    }

    /// The cost of a step from one node to another and the point it
    /// reaches: from the start, the goal's cost; into the end, nothing;
    /// otherwise the mean of the two rectangles' costs, rounded up, times
    /// one more than the distance to the middle of their shared edge.
    fn transition(&self, from: usize, to: usize) -> Result<(Ext, Point), Trap> {
        let a = &self.nodes[from];
        let b = &self.nodes[to];
        if from == 0 {
            let goal = b.goal.ok_or_else(|| limit("a goal"))?;
            return Ok((Ext::from_i32(self.query.goals[goal].0[4]), a.point));
        }
        if to == 1 {
            return Ok((Ext::ZERO, a.point));
        }
        let (ra, rb) = match (a.region, b.region) {
            (Some(ra), Some(rb)) => (ra, rb),
            _ => return Err(limit("a node")),
        };
        let point = if ra == rb {
            a.point
        } else {
            center(edge(self.regions[ra], self.regions[rb]))
        };
        let mean = Ext::from_i32(a.cost.wrapping_add(b.cost).wrapping_add(2) >> 1);
        let cost = (mean * (length(a.point, point) + Ext::from_i32(1)).single()).single();
        Ok((cost, point))
    }
}

/// Runs a search to its end or its step limit.
pub(crate) fn search(query: Query<'_>) -> Result<Found, Trap> {
    let mut graph = Graph {
        query,
        regions: vec![],
        nodes: vec![],
        goal_nodes: vec![],
        start_nodes: vec![],
    };
    graph.init();
    let steps = graph.query.steps;
    let mut open = vec![0];
    let mut closed: Vec<usize> = vec![];
    let mut current = 0;
    let mut iterations = 0;
    let mut goal = None;
    let mut ids = vec![];
    let mut points = vec![];
    loop {
        if iterations >= steps {
            break;
        }
        iterations += 1;
        if iterations > 65536 {
            return Err(limit("the search's steps"));
        }
        if current == 1 {
            let mut reverse = vec![];
            let mut node = Some(current);
            while let Some(id) = node {
                reverse.push(id);
                if goal.is_none() {
                    goal = graph.nodes[id].goal;
                }
                node = graph.nodes[id].parent;
            }
            let mut previous = None;
            for &id in reverse.iter().rev() {
                let region = graph.nodes[id].region;
                if region.is_some() && region != previous {
                    ids.push(id);
                    previous = region;
                }
            }
            points.extend(ids.iter().map(|&id| graph.nodes[id].point));
            if let Some(g) = goal {
                points.push(center(graph.query.goals[g]));
            }
            break;
        }
        for next in graph.neighbors(current)? {
            let (cost, point) = graph.transition(current, next)?;
            let cost = (cost + graph.nodes[current].g.ext()).single();
            if let Some(i) = closed.iter().position(|&id| id == next) {
                if graph.nodes[next].g.ext() <= cost {
                    continue;
                }
                closed.remove(i);
            }
            if open.contains(&next) {
                if cost >= graph.nodes[next].g.ext() {
                    continue;
                }
            } else {
                open.push(next);
            }
            let h = if next == 1 {
                Ext::ZERO
            } else {
                let goal = graph.nodes[next].goal.ok_or_else(|| limit("a goal"))?;
                length(point, center(graph.query.goals[goal]))
            };
            let n = &mut graph.nodes[next];
            n.g = cost.to_f32();
            n.f = (cost + h).single().to_f32();
            n.point = point;
            n.parent = Some(current);
        }
        if let Some(at) = open.iter().position(|&id| id == current) {
            open.remove(at);
        }
        closed.push(current);
        if open.is_empty() {
            break;
        }
        // The first node with the least estimate.
        let mut best = open[0];
        for &id in &open[1..] {
            if graph.nodes[id].f.ext() < graph.nodes[best].f.ext() {
                best = id;
            }
        }
        current = best;
    }
    let mut pass = 0;
    loop {
        if iterations >= steps {
            break;
        }
        iterations += 1;
        let mut changed = false;
        loop {
            let step = (100 / ((pass + 1) * (pass + 1))).max(1);
            for i in 1..ids.len() {
                let (Some(ra), Some(rb)) =
                    (graph.nodes[ids[i - 1]].region, graph.nodes[ids[i]].region)
                else {
                    return Err(limit("a node"));
                };
                let bounds = edge(graph.regions[ra], graph.regions[rb]).0;
                let before = points[i - 1];
                let at = points[i];
                let after = points.get(i + 1).copied().unwrap_or(at);
                let mut best = at;
                let mut score = (length(before, at) + length(at, after)).single();
                for (axis, sign) in [(0, 1), (0, -1), (1, 1), (1, -1)] {
                    let value = at[axis];
                    if bounds[axis + 2] - bounds[axis] <= 2
                        || (sign == 1 && value >= bounds[axis + 2] - step)
                        || (sign == -1 && bounds[axis] + step >= value)
                    {
                        continue;
                    }
                    let mut candidate = at;
                    candidate[axis] += sign * step;
                    let cost = (length(before, candidate) + length(candidate, after)).single();
                    if cost < score {
                        score = cost;
                        best = candidate;
                        changed = true;
                    }
                }
                points[i] = best;
            }
            if changed {
                break;
            }
            pass += 1;
            if step == 1 {
                break;
            }
        }
        if !changed {
            break;
        }
        if iterations > 65536 {
            return Err(limit("the path's smoothing"));
        }
    }
    Ok(Found {
        goal,
        points,
        iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query<'a>(goals: &'a [Region], fixed: &'a [Region]) -> Query<'a> {
        Query {
            start: [20, 20],
            goals,
            dynamic: &[],
            fixed,
            maximum: Some(128),
            free_cost: 3,
            steps: 1000,
        }
    }

    #[test]
    fn an_open_field_gives_a_straight_path() {
        let border = [
            Region([-7, -7, 7, 300, HARD]),
            Region([-7, -7, 300, 7, HARD]),
            Region([200, -7, 300, 300, HARD]),
            Region([-7, 200, 300, 300, HARD]),
        ];
        let goals = [Region([100, 20, 101, 21, 0])];
        let found = search(query(&goals, &border)).unwrap();
        assert_eq!(found.goal, Some(0));
        assert_eq!(found.points.first(), Some(&[20, 20]));
        assert_eq!(found.points.last(), Some(&[100, 20]));
    }

    #[test]
    fn a_wall_with_a_gap_is_passed_through_the_gap() {
        // A wall across the field at x 100 to 104, open from y 150 to 200.
        let fixed = [
            Region([-7, -7, 7, 300, HARD]),
            Region([-7, -7, 300, 7, HARD]),
            Region([250, -7, 300, 300, HARD]),
            Region([-7, 250, 300, 300, HARD]),
            Region([100, 0, 104, 150, HARD]),
        ];
        let goals = [Region([180, 20, 181, 21, 0])];
        let found = search(query(&goals, &fixed)).unwrap();
        assert_eq!(found.goal, Some(0));
        assert_eq!(found.points.last(), Some(&[180, 20]));
        // Some point lies in the gap, below the wall's end.
        assert!(
            found
                .points
                .iter()
                .any(|p| (99..=105).contains(&p[0]) && p[1] >= 150)
        );
    }

    #[test]
    fn a_goal_inside_a_hard_obstacle_is_never_reached() {
        let fixed = [Region([90, 0, 110, 40, HARD])];
        let goals = [Region([100, 20, 101, 21, 0])];
        let found = search(query(&goals, &fixed)).unwrap();
        assert_eq!(found.goal, None);
        assert!(found.points.is_empty());
    }
}
