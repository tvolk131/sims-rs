//! Original goal-labelled rectangle graph and A* search. The historical fixture
//! supplies a pointer at a protected gap. The later playable-capture comparison
//! in native_route_geometry executes that operation without substitution; see
//! docs/routing-gap-verification.md for the distinct evidence scopes.
use crate::{
    Point,
    regions::{Region, Space},
};

#[derive(Debug, Clone)]
struct Node {
    goal: Option<usize>,
    region: Option<usize>,
    neighbors: Option<Vec<usize>>,
    cost: i32,
    point: Point,
    parent: Option<usize>,
    f: f32,
    g: f32,
}
impl Node {
    fn new(goal: Option<usize>, region: Option<usize>, cost: i32) -> Self {
        Self {
            goal,
            region,
            cost,
            neighbors: None,
            point: Point::new(0, 0),
            parent: None,
            f: 0.,
            g: 0.,
        }
    }
}
pub struct Query<'a> {
    pub start: Point,
    pub goals: &'a [Region],
    pub dynamic: &'a [Region],
    pub fixed: &'a [Region],
    pub maximum: Option<i32>,
    pub aspect: Option<i32>,
    /// Original configuration+2c, assigned only to newly discovered free
    /// neighbor regions. Initial start/goal regions retain their own costs.
    pub free_cost: i32,
    pub smooth: bool,
    /// Native wrapper budget counts both graph search and path adjustment.
    pub steps: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Result {
    pub goal: Option<usize>,
    pub points: Vec<Point>,
    pub regions: Vec<Region>,
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
    Point::new((r.0[0] + r.0[2]) / 2, (r.0[1] + r.0[3]) / 2)
}
fn length(a: Point, b: Point) -> f32 {
    let x = (a.x - b.x) as f32;
    let y = (a.y - b.y) as f32;
    ((x * x + y * y) as f64).sqrt() as f32
}
fn edge(a: Region, b: Region) -> Region {
    Region([
        a.0[0].max(b.0[0]) - 1,
        a.0[1].max(b.0[1]) - 1,
        a.0[2].min(b.0[2]) + 1,
        a.0[3].min(b.0[3]) + 1,
        0,
    ])
}
impl Graph<'_> {
    fn space(&self) -> Space<'_> {
        Space {
            dynamic: self.query.dynamic,
            fixed: self.query.fixed,
            regions: &self.regions,
            maximum: self.query.maximum,
            aspect: self.query.aspect,
        }
    }
    fn obstacle(&self, rect: Region) -> Option<Region> {
        self.space().obstacle(rect).map(|i| {
            self.query
                .dynamic
                .iter()
                .chain(self.query.fixed)
                .nth(i)
                .copied()
                .unwrap()
        })
    }
    fn region_at(&self, rect: Region) -> Option<usize> {
        self.regions.iter().position(|r| r.overlaps(rect))
    }
    fn init(&mut self) {
        let start = self.query.start;
        let mut region = Region([start.x, start.y, start.x + 1, start.y + 1, 0]);
        self.space().expand(&mut region);
        self.regions.push(region);
        self.nodes.push(Node::new(None, Some(0), 0));
        self.nodes[0].point = start;
        self.nodes.push(Node::new(None, None, 0));
        for (i, &goal) in self.query.goals.iter().enumerate() {
            let obstacle = self.obstacle(goal);
            if obstacle.is_some_and(|r| r.0[4] == i32::MAX) {
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
    fn neighbors(&mut self, id: usize) -> std::result::Result<Vec<usize>, String> {
        if let Some(n) = &self.nodes[id].neighbors {
            return Ok(n.clone());
        }
        let mut neighbors = vec![];
        if id == 0 {
            neighbors = self.start_nodes.clone();
        } else if id != 1 {
            let region = self.regions[self.nodes[id].region.unwrap()];
            let [x0, y0, x1, y1, _] = region.0;
            let mut seed = Region([x0, y0 - 1, x0 + 1, y0, 0]);
            let mut side = 0;
            let mut steps = 0;
            while side < 4 {
                steps += 1;
                if steps > 8192 || self.nodes.len() > 65536 {
                    return Err("native rectangle graph diagnostic limit".into());
                }
                let obstacle = self.obstacle(seed);
                let bounds = if let Some(hard) = obstacle.filter(|r| r.0[4] == i32::MAX) {
                    hard
                } else {
                    let region = if let Some(i) = self.region_at(seed) {
                        i
                    } else {
                        let mut r = seed;
                        // Original 0x469eba prepares this local rectangle's
                        // address. Recovered gameplay bytes resolve the earlier
                        // supplied-pointer premise; wrapper parity is separate.
                        if !self.space().expand(&mut r) {
                            return Err("native neighbor expansion rejected a free seed".into());
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
                    3 => (1, y0 - seed.0[1] - 1, true),
                    _ => unreachable!(),
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
    fn transition(&self, from: usize, to: usize) -> (f32, Point) {
        let a = &self.nodes[from];
        let b = &self.nodes[to];
        if from == 0 {
            return (self.query.goals[b.goal.unwrap()].0[4] as f32, a.point);
        }
        if to == 1 {
            return (0., a.point);
        }
        let point = if a.region == b.region {
            a.point
        } else {
            center(edge(
                self.regions[a.region.unwrap()],
                self.regions[b.region.unwrap()],
            ))
        };
        let cost = ((a.cost + b.cost + 2) >> 1) as f32 * (length(a.point, point) + 1.);
        (cost, point)
    }
}

pub fn search(query: Query<'_>) -> std::result::Result<Result, String> {
    let mut graph = Graph {
        query,
        regions: vec![],
        nodes: vec![],
        goal_nodes: vec![],
        start_nodes: vec![],
    };
    graph.init();
    let mut open = vec![0];
    let mut closed = vec![];
    let mut current = 0;
    let mut iterations = 0;
    let mut goal = None;
    let mut ids = vec![];
    let mut points = vec![];
    loop {
        if graph.query.steps.is_some_and(|limit| iterations >= limit) {
            break;
        }
        iterations += 1;
        if iterations > 65536 {
            return Err("native rectangle search diagnostic limit".into());
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
            let (cost, point) = graph.transition(current, next);
            let cost = cost + graph.nodes[current].g;
            if let Some(i) = closed.iter().position(|&id| id == next) {
                if graph.nodes[next].g <= cost {
                    continue;
                }
                closed.remove(i);
            }
            if open.contains(&next) {
                if cost >= graph.nodes[next].g {
                    continue;
                }
            } else {
                open.push(next);
            }
            let h = if next == 1 {
                0.
            } else {
                length(
                    point,
                    center(graph.query.goals[graph.nodes[next].goal.unwrap()]),
                )
            };
            let n = &mut graph.nodes[next];
            n.g = cost;
            n.f = cost + h;
            n.point = point;
            n.parent = Some(current);
        }
        open.remove(open.iter().position(|&id| id == current).unwrap());
        closed.push(current);
        if open.is_empty() {
            break;
        }
        current = *open
            .iter()
            .min_by(|&&a, &&b| graph.nodes[a].f.total_cmp(&graph.nodes[b].f))
            .unwrap();
    }
    if graph.query.smooth {
        let mut pass = 0;
        loop {
            if graph.query.steps.is_some_and(|limit| iterations >= limit) {
                break;
            }
            iterations += 1;
            let mut changed = false;
            loop {
                let step = (100 / ((pass + 1) * (pass + 1))).max(1);
                for i in 1..ids.len() {
                    let bounds = edge(
                        graph.regions[graph.nodes[ids[i - 1]].region.unwrap()],
                        graph.regions[graph.nodes[ids[i]].region.unwrap()],
                    )
                    .0;
                    let before = points[i - 1];
                    let at = points[i];
                    let after = points.get(i + 1).copied().unwrap_or(at);
                    let mut best = at;
                    let mut score = length(before, at) + length(at, after);
                    for (axis, sign) in [(0, 1), (0, -1), (1, 1), (1, -1)] {
                        let value = if axis == 0 { at.x } else { at.y };
                        if bounds[axis + 2] - bounds[axis] <= 2
                            || (sign == 1 && value >= bounds[axis + 2] - step)
                            || (sign == -1 && bounds[axis] + step >= value)
                        {
                            continue;
                        }
                        let mut candidate = at;
                        if axis == 0 {
                            candidate.x += sign * step;
                        } else {
                            candidate.y += sign * step;
                        }
                        let cost = length(before, candidate) + length(candidate, after);
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
                return Err("native path adjustment diagnostic limit".into());
            }
        }
    }
    Ok(Result {
        goal,
        points,
        regions: graph.regions,
        iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn matches_native_search_with_explicit_pointer_boundary() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/rectangle-path-supplied-pointer.json"
        ))
        .unwrap();
        assert_eq!(fixture["supplied_stack_pointer"], true);
        for (i, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
            let c = &row["input"];
            let o = &row["original"];
            let goals: Vec<Region> = serde_json::from_value(c["goals"].clone()).unwrap();
            let fixed: Vec<Region> = serde_json::from_value(c["obstacles"].clone()).unwrap();
            let dynamic: Vec<Region> =
                serde_json::from_value(c.get("dynamic").cloned().unwrap_or(serde_json::json!([])))
                    .unwrap();
            let r = search(Query {
                start: Point::new(
                    c["start"][0].as_i64().unwrap() as i32,
                    c["start"][1].as_i64().unwrap() as i32,
                ),
                goals: &goals,
                fixed: &fixed,
                dynamic: &dynamic,
                maximum: c["bounded"]
                    .as_bool()
                    .unwrap_or(true)
                    .then_some(c["maximum"].as_i64().unwrap_or(64) as i32),
                aspect: c["aspect"].as_bool().unwrap_or(false).then_some(3),
                free_cost: c["free_cost"].as_i64().unwrap_or(0) as i32,
                smooth: c["smooth"].as_bool().unwrap(),
                steps: c["limit"].as_u64().map(|n| n as usize),
            })
            .unwrap();
            assert_eq!(
                r.goal.map_or(-1, |g| g as i64),
                o["goal"].as_i64().unwrap(),
                "case {i}"
            );
            let path: Vec<[i32; 2]> = serde_json::from_value(o["path"].clone()).unwrap();
            assert_eq!(
                r.points,
                path.into_iter()
                    .map(|p| Point::new(p[0], p[1]))
                    .collect::<Vec<_>>(),
                "case {i}"
            );
            let regions: Vec<Region> = serde_json::from_value(o["rectangles"].clone()).unwrap();
            assert_eq!(r.regions, regions, "case {i}");
            assert_eq!(
                r.iterations,
                o["iterations"].as_u64().unwrap() as usize,
                "case {i}"
            );
        }
    }
}
