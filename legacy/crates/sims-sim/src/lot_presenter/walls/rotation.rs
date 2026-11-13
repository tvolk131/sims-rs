//! 5404b5 rotates shared edges before corners, then sorts both floor vectors.
use super::{BASIS, cap, construction::Graph, grid};

pub(crate) const DIRECTIONS: [[i8; 2]; 8] = [
    [0, -1],
    [0, 1],
    [-1, 0],
    [1, 0],
    [-1, -1],
    [1, 1],
    [1, -1],
    [-1, 1],
];
pub(crate) fn key([x, y, _]: [i8; 3]) -> (i32, i32) {
    (i32::from(x) + i32::from(y), i32::from(x) - i32::from(y))
}
fn point([x, y, z]: [i8; 3], delta: u8) -> [i8; 3] {
    match delta {
        1 => [64 - y, x, z],
        2 => [64 - x, 64 - y, z],
        3 => [y, 64 - x, z],
        _ => [x, y, z],
    }
}
/// 53f1db, including its half-distance truncation and strict/equal choices.
fn heading(a: [i8; 3], b: [i8; 3]) -> i32 {
    if a == b {
        return 8;
    }
    let dx = i32::from(b[0]) - i32::from(a[0]);
    let dy = i32::from(b[1]) - i32::from(a[1]);
    let (x, y, minus, plus) = (dx.abs(), dy.abs(), (dx - dy).abs() / 2, (dx + dy).abs() / 2);
    if x < y && x <= minus && x <= plus {
        i32::from(dy >= 0)
    } else if y < x && y <= minus && y <= plus {
        2 + i32::from(dx >= 0)
    } else if minus < x && minus < y && minus < plus {
        4 + i32::from(dx + dy >= 0)
    } else {
        6 + i32::from(dx - dy < 0)
    }
}
impl Graph {
    pub fn validate(&self) -> Result<(), String> {
        self.state.validate()?;
        if self.rotation > 3
            || self.geometry.positions.len() != self.state.corners.len()
            || self.geometry.rooms.len() != self.state.edges.len()
            || self
                .geometry
                .positions
                .iter()
                .any(|p| !(1..=2).contains(&p[2]))
            || self
                .state
                .edges
                .iter()
                .any(|e| !(0..8).contains(&e.kind) || e.ends[0] == e.ends[1])
        {
            return Err("invalid native wall graph geometry".into());
        }
        let unique: std::collections::BTreeSet<_> =
            self.geometry.positions.iter().copied().collect();
        if unique.len() != self.geometry.positions.len() {
            return Err("duplicate native wall corner position".into());
        }
        for e in &self.state.edges {
            let a = self.geometry.positions[e.ends[0]];
            let b = self.geometry.positions[e.ends[1]];
            if (0..2).any(|n| {
                i32::from(b[n]) - i32::from(a[n])
                    != i32::from(DIRECTIONS[e.kind as usize][n]) * e.segments.len() as i32
            }) {
                return Err("native wall edge direction or segment extent mismatch".into());
            }
        }
        let mut corners = vec![false; self.state.corners.len()];
        let mut edges = vec![false; self.state.edges.len()];
        for (f, floor) in self.state.floors.iter().enumerate() {
            for &id in &floor.corners {
                if std::mem::replace(&mut corners[id], true)
                    || self.geometry.positions[id][2] != f as i8 + 1
                {
                    return Err("invalid native wall graph corner ownership".into());
                }
            }
            for &id in &floor.edges {
                if std::mem::replace(&mut edges[id], true)
                    || self.state.edges[id]
                        .ends
                        .iter()
                        .any(|&c| self.geometry.positions[c][2] != f as i8 + 1)
                {
                    return Err("invalid native wall graph edge ownership".into());
                }
            }
        }
        if corners.contains(&false) || edges.contains(&false) {
            return Err("unowned native wall graph record".into());
        }
        Ok(())
    }

    pub fn rotate(&mut self, rotation: u8) -> Result<(), String> {
        self.validate()?;
        if rotation > 3 {
            return Err("native wall graph rotation outside table".into());
        }
        let delta = rotation.wrapping_sub(self.rotation) & 3;
        if delta != 0
            && self
                .geometry
                .positions
                .iter()
                .any(|p| !(1..64).contains(&p[0]) || !(1..64).contains(&p[1]))
        {
            return Err("native wall corner outside rotation table interior".into());
        }
        self.rotation = rotation;
        for floor in &self.state.floors {
            for &id in &floor.edges {
                // 53d93a skips its entire body when delta is zero.
                if delta == 0 {
                    continue;
                }
                let e = &mut self.state.edges[id];
                for bit in &mut e.bits {
                    *bit = grid::rotate(*bit, delta)?;
                }
                let mut a = point(self.geometry.positions[e.ends[0]], delta);
                let mut b = point(self.geometry.positions[e.ends[1]], delta);
                if key(b) < key(a) {
                    std::mem::swap(&mut a, &mut b);
                    e.ends.swap(0, 1);
                    e.bits.swap(0, 1);
                    e.segments.reverse();
                    for s in &mut e.segments {
                        s.patterns.swap(0, 1);
                    }
                    self.geometry.rooms[id].swap(0, 1);
                }
                e.kind = heading(a, b);
            }
            for &id in &floor.corners {
                self.geometry.positions[id] = point(self.geometry.positions[id], delta);
                let c = &mut self.state.corners[id];
                c.base = grid::rotate(c.base, delta)?;
                for mask in &mut c.masks {
                    *mask = grid::rotate(*mask, delta)?;
                }
                c.edges.rotate_left(delta as usize);
                // Unlike53cc00, corner rotation supplies +c to BOTH cap calls.
                c.valid[0] = u8::from(cap(
                    c.masks[1],
                    c.masks[2],
                    c.masks[3],
                    BASIS,
                    &mut c.caps[0],
                ));
                c.valid[1] = u8::from(cap(
                    c.masks[0],
                    c.masks[2],
                    c.masks[3],
                    BASIS,
                    &mut c.caps[1],
                ));
            }
        }
        let starts: Vec<_> = self
            .state
            .edges
            .iter()
            .map(|e| self.geometry.positions[e.ends[0]])
            .collect();
        for floor in &mut self.state.floors {
            sims_vm::presenter::sort_wall_records(&mut floor.corners, &self.geometry.positions)?;
            sims_vm::presenter::sort_wall_records(&mut floor.edges, &starts)?;
        }
        Ok(())
    }
}
