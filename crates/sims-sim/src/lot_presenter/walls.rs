//! Native54051c wall-record flags and corner caps. Record construction from the
//! loaded wall grid and consumption by the rasterizer are separate services.
use serde::{Deserialize, Serialize};

pub mod construction;
pub mod cutaway;
pub mod grid;
pub mod rotation;

/// Original407650 initializes these two entries of the eight-direction table.
pub const BASIS: [[u8; 3]; 2] = [[0, 255, 0], [255, 0, 0]];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub flags: u8,
    pub style: u8,
    pub patterns: [u16; 2],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub kind: i32,
    pub ends: [usize; 2],
    pub bits: [u8; 2],
    pub segments: Vec<Segment>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Corner {
    /// +8; masks are +9,+a,+b,+c in order.
    pub base: u8,
    pub masks: [u8; 4],
    pub edges: [Option<usize>; 4],
    /// +24/+25 guard the two native descriptors. A false result preserves
    /// descriptor bytes; even a successful write leaves padding untouched.
    pub valid: [u8; 2],
    pub caps: [[u8; 20]; 2],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Floor {
    pub corners: Vec<usize>,
    pub edges: Vec<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub corners: Vec<Corner>,
    pub edges: Vec<Edge>,
    pub floors: [Floor; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Edge(usize),
    Corner(usize),
    Caps(usize),
}
fn solid(style: u8) -> bool {
    !matches!(style, 255 | 2 | 12 | 13 | 14)
}

/// 53cd4e. Basis bytes are original6e486c/6e4872 inputs, not screen pixels.
pub fn cap(
    mut mask: u8,
    excluded: u8,
    included: u8,
    basis: [[u8; 3]; 2],
    out: &mut [u8; 20],
) -> bool {
    mask &= !excluded;
    if mask == 0 {
        return false;
    }
    let (axis, kind, direction, bits, side) = match mask | included {
        1 => (1, 3, 0, 2, 0),
        2 => (0, 3, 1, 1, 1),
        4 | 6 => (0, 3, 2, 1, 1),
        8 | 9 => (1, 3, 3, 2, 0),
        0x14 => (0, 2, 2, 1, 1),
        0x18 => (1, 2, 3, 2, 0),
        0x30 => (0, 2, 1, 16, 1),
        0x90 => (1, 2, 3, 16, 0),
        _ => return false,
    };
    out[..2].copy_from_slice(&basis[axis][..2]);
    out[2] = 0;
    for (offset, value) in [(4, kind), (8, direction), (12, bits)] {
        out[offset..offset + 4].copy_from_slice(&i32::to_le_bytes(value));
    }
    out[16] = side;
    true
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        if self.corners.len() > 32768
            || self.edges.len() > 32768
            || self.edges.iter().any(|e| {
                e.segments.is_empty()
                    || e.segments.len() > 4096
                    || e.ends.iter().any(|i| *i >= self.corners.len())
            })
            || self.corners.iter().enumerate().any(|(i, c)| {
                c.edges
                    .iter()
                    .flatten()
                    .any(|e| self.edges.get(*e).is_none_or(|e| !e.ends.contains(&i)))
            })
            || self.floors.iter().any(|f| {
                f.corners.iter().any(|i| *i >= self.corners.len())
                    || f.edges.iter().any(|i| *i >= self.edges.len())
            })
        {
            return Err("invalid native wall presentation records".into());
        }
        Ok(())
    }
    fn endpoint(&self, edge: usize, corner: usize) -> (usize, u8) {
        let e = &self.edges[edge];
        if e.ends[0] == corner {
            (0, e.bits[0])
        } else {
            (e.segments.len() - 1, e.bits[1])
        }
    }
    fn prepare_edge(&mut self, id: usize, enabled: u8) {
        for s in &mut self.edges[id].segments {
            s.flags = 0;
            if enabled != 0 {
                if solid(s.style) {
                    s.flags |= 1;
                }
                if matches!(s.style, 3 | 5 | 6 | 15 | 22 | 23) {
                    s.flags |= 8;
                }
            }
        }
    }
    fn prepare_corner(&mut self, id: usize) {
        self.corners[id].masks = [0, self.corners[id].base, 0, 0];
        let edges = self.corners[id].edges;
        let mut active = [false; 4];
        let mut walls = [false; 4];
        for (slot, edge) in edges.iter().enumerate() {
            if let Some(e) = edge {
                let (s, _) = self.endpoint(*e, id);
                let s = &self.edges[*e].segments[s];
                active[slot] = s.flags & 1 != 0;
                walls[slot] = solid(s.style);
            }
        }
        for (slot, edge) in edges.iter().enumerate() {
            if !active[slot] {
                continue;
            }
            let edge = edge.unwrap();
            let (s, bit) = self.endpoint(edge, id);
            if (0..4).all(|other| {
                other == slot || edges[other].is_none() || active[other] || !walls[other]
            }) {
                let c = &mut self.corners[id];
                if c.masks[1] & bit != 0 {
                    c.masks[0] |= bit;
                    c.masks[1] &= !bit;
                }
            } else {
                let flags = &mut self.edges[edge].segments[s].flags;
                if *flags & 6 == 0 {
                    // Original slot0/3 choose bit2 for active segments;
                    // slot1/2 choose bit4. Preserve reads of the live flag.
                    let bit2 = if slot == 0 || slot == 3 {
                        *flags & 1 != 0
                    } else {
                        *flags & 1 == 0
                    };
                    if bit2 {
                        if *flags & 4 == 0 {
                            *flags |= 2;
                        }
                    } else if *flags & 2 == 0 {
                        *flags |= 4;
                    }
                }
            }
        }
    }
    fn prepare_caps(&mut self, id: usize, basis: [[u8; 3]; 2]) {
        for edge in self.corners[id].edges.into_iter().flatten() {
            let (s, bit) = self.endpoint(edge, id);
            let e = &self.edges[edge];
            let s = &e.segments[s];
            let c = &mut self.corners[id];
            if s.flags & 6 == 0 {
                if matches!(s.style, 2 | 12 | 13 | 14) {
                    c.masks[2] |= bit;
                }
            } else {
                match e.kind {
                    0..=3 => c.masks[3] |= bit,
                    6 if s.flags & 2 != 0 => c.masks[3] |= bit,
                    7 if s.flags & 4 != 0 => c.masks[3] |= bit,
                    6 | 7 => c.masks[2] |= bit,
                    _ => (),
                }
            }
            if s.flags & 1 != 0 && matches!(s.style, 3 | 5 | 6 | 15 | 23) {
                c.masks[2] |= bit;
            }
        }
        let c = &mut self.corners[id];
        c.valid[0] = u8::from(cap(
            c.masks[1],
            c.masks[2],
            c.masks[3],
            basis,
            &mut c.caps[0],
        ));
        c.valid[1] = u8::from(cap(c.masks[0], c.masks[2], 0, basis, &mut c.caps[1]));
    }
    /// Level zero visits both floors. Each floor finishes all three passes
    /// before the next floor starts; vector order is retained exactly.
    pub fn update(
        &mut self,
        enabled: u8,
        level: i32,
        basis: [[u8; 3]; 2],
        mut visit: impl FnMut(Event),
    ) -> Result<(), String> {
        self.validate()?;
        let floors = match level {
            0 => 0..2,
            1 => 0..1,
            2 => 1..2,
            _ => return Err("native wall presentation floor unavailable".into()),
        };
        for f in floors {
            for n in 0..self.floors[f].edges.len() {
                let id = self.floors[f].edges[n];
                visit(Event::Edge(id));
                self.prepare_edge(id, enabled);
            }
            for n in 0..self.floors[f].corners.len() {
                let id = self.floors[f].corners[n];
                visit(Event::Corner(id));
                self.prepare_corner(id);
            }
            for n in 0..self.floors[f].corners.len() {
                let id = self.floors[f].corners[n];
                visit(Event::Caps(id));
                self.prepare_caps(id, basis);
            }
        }
        Ok(())
    }
}
