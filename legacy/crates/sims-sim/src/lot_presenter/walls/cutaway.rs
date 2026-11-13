//! 54059a cutaway flags/caps followed by53edde's ordered tile invalidations.
//! Mask production and dirty topology rebuilding are explicit caller services.
use super::{BASIS, Segment, construction::Graph, rotation::DIRECTIONS, solid};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mask {
    /// Row of the first supplied64-bit word. Explicit guard rows may describe
    /// native storage preceding/following the64-row mask at view+208.
    pub first_row: i32,
    pub rows: Vec<u64>,
}
impl Mask {
    fn contains(&self, [x, y, _]: [i8; 3]) -> Result<bool, String> {
        let index = i32::from(y)
            .checked_sub(self.first_row)
            .and_then(|n| usize::try_from(n).ok());
        let row = index
            .and_then(|n| self.rows.get(n))
            .ok_or("native cutaway mask row unavailable")?;
        // MSVC __allshl uses CL; a signed negative byte becomes a shift>=64.
        Ok((0..64).contains(&x) && row & (1u64 << x as u32) != 0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub segments: Vec<Vec<Segment>>,
}
impl History {
    /// Clean-manager branch of540d38/53edd1. Rebuild flags are owned by the
    /// caller; a topology change requires its native reconstruction boundary.
    pub fn capture(graph: &Graph) -> Result<Self, String> {
        graph.validate()?;
        Ok(Self {
            segments: graph
                .state
                .edges
                .iter()
                .map(|e| e.segments.clone())
                .collect(),
        })
    }
    pub fn validate(&self, graph: &Graph) -> Result<(), String> {
        if self.segments.len() != graph.state.edges.len()
            || self
                .segments
                .iter()
                .zip(&graph.state.edges)
                .any(|(a, b)| a.len() != b.segments.len())
        {
            return Err("native cutaway history does not match wall records".into());
        }
        Ok(())
    }
}
fn add(mut p: [i8; 3], [x, y]: [i8; 2]) -> [i8; 3] {
    p[0] = p[0].wrapping_add(x);
    p[1] = p[1].wrapping_add(y);
    p
}
fn tile(p: [i8; 3], kind: i32) -> [i8; 3] {
    add(
        p,
        match kind {
            0 | 6 => [0, -1],
            2 | 7 => [-1, 0],
            4 => [-1, -1],
            _ => [0, 0],
        },
    )
}
fn flags(graph: &Graph, id: usize, mask: &Mask) -> Result<Vec<u8>, String> {
    let e = &graph.state.edges[id];
    let bit = match e.kind {
        2 | 3 => 2,
        4 | 5 => 32,
        6 | 7 => 16,
        _ => 1,
    };
    let mut p = graph.geometry.positions[e.ends[0]];
    let mut result = Vec::with_capacity(e.segments.len());
    for s in &e.segments {
        let mut active = mask.contains(p)? && solid(s.style);
        if active {
            active = match bit {
                16 => {
                    let t = tile(p, e.kind);
                    mask.contains(add(t, [-1, 0]))? || mask.contains(add(t, [0, -1]))?
                }
                // Preserve the original two-cell offsets and AND test.
                32 => {
                    let t = tile(p, e.kind);
                    mask.contains(add(t, [0, -2]))? && mask.contains(add(t, [-2, 0]))?
                }
                1 => mask.contains(add(p, [-1, 0]))?,
                _ => mask.contains(add(p, [0, -1]))?,
            };
        }
        result.push(
            u8::from(active)
                | if active && matches!(s.style, 3 | 5 | 6 | 15 | 22 | 23) {
                    8
                } else {
                    0
                },
        );
        p = add(p, DIRECTIONS[e.kind as usize]);
    }
    let (before, after) = if matches!(e.kind, 3 | 6) {
        (4, 2)
    } else {
        (2, 4)
    };
    for n in 0..result.len().saturating_sub(1) {
        if (result[n] ^ result[n + 1]) & 1 == 0 {
            continue;
        }
        if result[n] & 1 == 0 {
            if solid(e.segments[n].style) {
                result[n + 1] |= after;
            }
        } else if solid(e.segments[n + 1].style) {
            result[n] |= before;
        }
    }
    Ok(result)
}

/// The calculation passes finish before any invalidation callback. Scanning
/// retains live reads of subsequent segments across each acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    floor: usize,
    rotation: u8,
    edge: usize,
    segment: usize,
    at: Option<[i8; 3]>,
    pending: bool,
    complete: bool,
}
impl Cursor {
    pub fn begin(
        graph: &mut Graph,
        history: &History,
        mask: &Mask,
        level: i32,
    ) -> Result<Self, String> {
        graph.validate()?;
        history.validate(graph)?;
        if !(1..=2).contains(&level) {
            return Err("native cutaway floor unavailable".into());
        }
        let floor = level as usize - 1;
        // Check supplied mask memory before writing; missing memory is a
        // host diagnostic, not a fabricated native zero row.
        let planned: Vec<_> = graph.state.floors[floor]
            .edges
            .iter()
            .map(|&id| Ok((id, flags(graph, id, mask)?)))
            .collect::<Result<_, String>>()?;
        for (id, values) in planned {
            for (s, value) in graph.state.edges[id].segments.iter_mut().zip(values) {
                s.flags = value;
            }
        }
        for n in 0..graph.state.floors[floor].corners.len() {
            graph
                .state
                .prepare_corner(graph.state.floors[floor].corners[n]);
        }
        for n in 0..graph.state.floors[floor].corners.len() {
            graph
                .state
                .prepare_caps(graph.state.floors[floor].corners[n], BASIS);
        }
        Ok(Self {
            floor,
            rotation: graph.rotation,
            edge: 0,
            segment: 0,
            at: None,
            pending: false,
            complete: false,
        })
    }
    pub fn validate(&self, graph: &Graph, history: &History) -> Result<(), String> {
        graph.validate()?;
        history.validate(graph)?;
        if self.floor > 1 || self.rotation != graph.rotation {
            return Err("invalid native cutaway cursor context".into());
        }
        let edges = &graph.state.floors[self.floor].edges;
        if self.edge > edges.len()
            || (self.pending && self.at.is_none())
            || (self.complete
                && (self.edge != edges.len()
                    || self.at.is_some()
                    || self.pending
                    || self.segment != 0))
        {
            return Err("invalid native cutaway cursor ownership".into());
        }
        if self.edge == edges.len() {
            if self.at.is_some() || self.pending || self.segment != 0 {
                return Err("cutaway cursor past final edge".into());
            }
        } else {
            let e = &graph.state.edges[edges[self.edge]];
            if self.segment > e.segments.len()
                || (self.pending && self.segment == e.segments.len())
                || (self.at.is_none() && self.segment != 0)
            {
                return Err("invalid native cutaway segment cursor".into());
            }
            if let Some(at) = self.at {
                let mut expected = graph.geometry.positions[e.ends[0]];
                for _ in 0..self.segment {
                    expected = add(expected, DIRECTIONS[e.kind as usize]);
                }
                if at != expected {
                    return Err("native cutaway cursor position changed".into());
                }
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, graph: &Graph, history: &History) -> Result<Option<[i8; 3]>, String> {
        self.validate(graph, history)?;
        if self.complete {
            return Ok(None);
        }
        if self.pending {
            return Ok(self.at);
        }
        loop {
            let Some(&id) = graph.state.floors[self.floor].edges.get(self.edge) else {
                self.complete = true;
                return Ok(None);
            };
            let e = &graph.state.edges[id];
            if self.at.is_none() {
                self.at = Some(graph.geometry.positions[e.ends[0]]);
            }
            if self.segment == e.segments.len() {
                self.edge += 1;
                self.segment = 0;
                self.at = None;
                continue;
            }
            if e.segments[self.segment] != history.segments[id][self.segment] {
                self.pending = true;
                return Ok(self.at);
            }
            self.next(graph);
        }
    }
    fn next(&mut self, graph: &Graph) {
        let id = graph.state.floors[self.floor].edges[self.edge];
        self.at = Some(add(
            self.at.unwrap(),
            DIRECTIONS[graph.state.edges[id].kind as usize],
        ));
        self.segment += 1;
    }
    pub fn acknowledge(
        &mut self,
        graph: &Graph,
        history: &History,
        at: [i8; 3],
    ) -> Result<(), String> {
        self.validate(graph, history)?;
        if !self.pending || self.at != Some(at) {
            return Err("native cutaway acknowledgement does not match callback".into());
        }
        self.next(graph);
        self.pending = false;
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
}
