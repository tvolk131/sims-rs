//! Where walls meet: which trim each corner takes, and the cap that closes
//! off an exposed wall end.
//!
//! This follows the original's wall records. A run is a straight line of
//! wall segments between two corner records; a corner record stands
//! wherever walls end or meet at an angle (0x53d2fc's tables, 0x53d4c4).
//! When the cutaway changes, the original:
//!
//! 1. sets bit 1 on every cut segment, then marks a cut segment whose
//!    neighbour along its run is standing (0x54059a);
//! 2. at each corner record, moves a cut wall from the standing set to the
//!    cut set when no standing wall meets it there, and otherwise marks it
//!    (0x53ca67);
//! 3. collects two masks per corner, walls that take no trim or cap there
//!    (fences, cut door-like walls) and marked walls, and turns the corner's
//!    walls into at most one cap (0x53cc00, 0x53cd4e).
//!
//! The marks are bits 2 and 4 of a segment's flags, one for each end:
//! along a run in the +y or (+1, +1) direction bit 2 marks the far end and
//! bit 4 the near one; in the +x or (+1, -1) direction it is the other way
//! round. Everything here is in view coordinates except the corner
//! records' layout, which the original builds from the unrotated lot.

use std::collections::BTreeMap;

/// One wall segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wall {
    /// Its two ends as lot corners.
    pub lot: [[i32; 2]; 2],
    /// The same ends in the view, in the same order.
    pub view: [[i32; 2]; 2],
    pub style: u16,
    /// Whether the cutaway cuts it down.
    pub cut: bool,
}

/// The line a run follows, by the original's run kinds (0x53bfb5 maps
/// them to the cap types 1, 2, 0x20 and 0x10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Line {
    /// Along view y: kind 1, type 1.
    Y,
    /// Along view x: kind 3, type 2.
    X,
    /// Along (1, 1): kind 5, type 0x20.
    Rising,
    /// Along (1, -1): kind 6, type 0x10.
    Falling,
}

/// The cap at a corner (the original's 0x53cd4e descriptor): a slice of
/// a phantom wall on `line` that touches the corner from the tile
/// `offset` away from it, `width` pixels wide at the far zoom (doubling
/// with each zoom), right of the corner when `right` and left of it
/// otherwise. Its pattern and wall height come from the wall in `slot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cap {
    pub offset: [i32; 2],
    pub width: i32,
    pub slot: usize,
    pub line: Line,
    pub right: bool,
}

/// What one corner draws.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Corner {
    /// The corner in the view.
    pub view: [i32; 2],
    /// The wall in each slot (records only): leaving the corner toward
    /// view -y or (-1, -1), -x or (-1, 1), +y or (1, 1), +x or (1, -1).
    pub slots: [Option<usize>; 4],
    /// Whether the original keeps a record here. Elsewhere a run passes
    /// straight through and its own drawing lays the trim.
    pub record: bool,
    /// Trim bits (see `trim_frame` in the scene) of the standing walls'
    /// trim and of the cut walls' trim.
    pub standing: u8,
    pub cut: u8,
    /// The cap, and whether it takes the cut wall shape.
    pub cap: Option<(Cap, bool)>,
}

/// The walls and corners of one level.
#[derive(Debug, Clone, Default)]
pub struct Corners {
    pub walls: Vec<Wall>,
    /// Each wall's flags: 1 cut, 2 and 4 marks, 8 a cut door-like style.
    pub flags: Vec<u8>,
    /// By lot corner.
    pub corners: BTreeMap<[i32; 2], Corner>,
}

/// Whether a style can be cut down (0x53d7b3): not a fence, railing or
/// missing wall.
pub fn solid(style: u16) -> bool {
    !matches!(style, 2 | 12 | 13 | 14 | 255)
}

/// Door-like styles, which take no trim when cut (0x53d7d9 without 22).
pub fn door_like(style: u16) -> bool {
    matches!(style, 3 | 5 | 6 | 15 | 23)
}

/// The bit for a wall leaving a corner in direction `d`: 1, 2, 4 and 8
/// for -y, -x, +y and +x, 0x10 to 0x80 for (-1, -1), (-1, 1), (1, 1) and
/// (1, -1).
pub fn bit(d: [i32; 2]) -> u8 {
    match d {
        [0, -1] => 1,
        [-1, 0] => 2,
        [0, 1] => 4,
        [1, 0] => 8,
        [-1, -1] => 0x10,
        [-1, 1] => 0x20,
        [1, 1] => 0x40,
        [1, -1] => 0x80,
        _ => 0,
    }
}

fn slot(bit: u8) -> Option<usize> {
    match bit {
        0x01 | 0x10 => Some(0),
        0x02 | 0x20 => Some(1),
        0x04 | 0x40 => Some(2),
        0x08 | 0x80 => Some(3),
        _ => None,
    }
}

/// The original's sort key for corners and runs.
fn key([x, y]: [i32; 2]) -> (i32, i32) {
    (x + y, x - y)
}

/// The corner-record test (0x53d4c4, 0x53d4ec, from 0x53d2fc's tables):
/// walls meet here at an angle, or one ends here. The validity test is
/// lopsided in the original and is kept so.
pub fn record(mask: u8) -> bool {
    let valid = !(mask & 16 != 0 && mask & 3 != 0
        || mask & 32 != 0 && mask & 6 != 0
        || mask & 64 != 0 && mask & 4 != 0
        || mask & 128 != 0 && mask & 9 != 0);
    let junction = mask & 1 != 0 && mask & 0xfa != 0
        || mask & 2 != 0 && mask & 0xf5 != 0
        || mask & 8 != 0 && mask & 0xf5 != 0
        || mask & 16 != 0 && mask & 0xaf != 0
        || mask & 64 != 0 && mask & 0xaf != 0
        || mask & 128 != 0 && mask & 0x5f != 0
        || mask & 32 != 0 && mask & 0x5f != 0;
    valid && (junction || mask.count_ones() == 1)
}

/// The cap descriptor (0x53cd4e) for the walls `walls` at a corner, less
/// `excluded`, plus `included`.
pub fn cap(walls: u8, excluded: u8, included: u8) -> Option<Cap> {
    let walls = walls & !excluded;
    if walls == 0 {
        return None;
    }
    const UP: [i32; 2] = [0, -1];
    const LEFT: [i32; 2] = [-1, 0];
    let (offset, width, slot, line, right) = match walls | included {
        1 => (LEFT, 3, 0, Line::X, false),
        2 => (UP, 3, 1, Line::Y, true),
        4 | 6 => (UP, 3, 2, Line::Y, true),
        8 | 9 => (LEFT, 3, 3, Line::X, false),
        0x14 => (UP, 2, 2, Line::Y, true),
        0x18 => (LEFT, 2, 3, Line::X, false),
        0x30 => (UP, 2, 1, Line::Falling, true),
        0x90 => (LEFT, 2, 3, Line::Falling, false),
        _ => return None,
    };
    Some(Cap {
        offset,
        width,
        slot,
        line,
        right,
    })
}

impl Wall {
    /// The line it follows and its ends in the original's run order (the
    /// lower key first).
    fn run(&self) -> (Line, [[i32; 2]; 2]) {
        let [a, b] = self.view;
        let (lo, hi) = if key(a) <= key(b) { (a, b) } else { (b, a) };
        let line = match [hi[0] - lo[0], hi[1] - lo[1]] {
            [0, _] => Line::Y,
            [_, 0] => Line::X,
            [x, y] if x == y => Line::Rising,
            _ => Line::Falling,
        };
        (line, [lo, hi])
    }

    /// The mark for its end at view corner `at`.
    fn mark(&self, at: [i32; 2]) -> u8 {
        let (line, [_, hi]) = self.run();
        let far = at == hi;
        match line {
            Line::Y | Line::Rising if far => 2,
            Line::Y | Line::Rising => 4,
            _ if far => 4,
            _ => 2,
        }
    }

    /// The line it follows in the view.
    pub fn line(&self) -> Line {
        self.run().0
    }

    /// Its other end, seen from view corner `at`.
    fn other(&self, at: [i32; 2]) -> [i32; 2] {
        if self.view[0] == at {
            self.view[1]
        } else {
            self.view[0]
        }
    }
}

impl Corners {
    pub fn new(walls: Vec<Wall>) -> Self {
        let mut flags: Vec<u8> = walls
            .iter()
            .map(|w| {
                if w.cut && solid(w.style) {
                    1 | if matches!(w.style, 3 | 5 | 6 | 15 | 22 | 23) {
                        8
                    } else {
                        0
                    }
                } else {
                    0
                }
            })
            .collect();
        // The walls at each corner, and the records.
        let mut at: BTreeMap<[i32; 2], (Vec<usize>, u8)> = BTreeMap::new();
        for (i, w) in walls.iter().enumerate() {
            for e in 0..2 {
                let o = 1 - e;
                let d = [w.lot[o][0] - w.lot[e][0], w.lot[o][1] - w.lot[e][1]];
                let entry = at.entry(w.lot[e]).or_default();
                entry.0.push(i);
                entry.1 |= bit(d);
            }
        }
        let mut corners = BTreeMap::new();
        for (&corner, (list, mask)) in &at {
            let view = walls[list[0]].view[usize::from(walls[list[0]].lot[1] == corner)];
            let mut c = Corner {
                view,
                record: record(*mask),
                ..Corner::default()
            };
            if c.record {
                for &i in list {
                    let d = walls[i].other(view);
                    if let Some(s) = slot(bit([d[0] - view[0], d[1] - view[1]])) {
                        c.slots[s].get_or_insert(i);
                    }
                }
            }
            corners.insert(corner, c);
        }
        // 1. A cut segment next to a standing one along its run.
        for (corner, (list, _)) in &at {
            let c = &corners[corner];
            if c.record {
                continue;
            }
            for &n in list {
                let (line, [_, hi]) = walls[n].run();
                if hi != c.view {
                    continue;
                }
                let next = list.iter().copied().find(|&m| {
                    let (l, [lo, _]) = walls[m].run();
                    m != n && l == line && lo == c.view
                });
                let Some(m) = next else {
                    continue;
                };
                let (a, b) = (flags[n] & 1 != 0, flags[m] & 1 != 0);
                if !a && b && solid(walls[n].style) {
                    flags[m] |= walls[m].mark(c.view);
                } else if a && !b && solid(walls[m].style) {
                    flags[n] |= walls[n].mark(c.view);
                }
            }
        }
        // Where a run passes straight through, the run's own drawing lays
        // the trim (0x53df0b onward, not read in full): standing walls'
        // trim, or the cut walls' when nothing there stands, matched
        // against the original's screen (2026-11-14).
        for (corner, (list, _)) in &at {
            let c = corners.get_mut(corner).unwrap();
            if c.record {
                continue;
            }
            for &i in list {
                let w = &walls[i];
                let d = w.other(c.view);
                let b = bit([d[0] - c.view[0], d[1] - c.view[1]]);
                if !solid(w.style) {
                    continue;
                }
                if !w.cut {
                    c.standing |= b;
                } else if !door_like(w.style) {
                    c.cut |= b;
                }
            }
            if c.standing != 0 {
                c.cut = 0;
            }
        }
        let mut order: Vec<[i32; 2]> = corners
            .iter()
            .filter(|(_, c)| c.record)
            .map(|(&k, _)| k)
            .collect();
        order.sort_by_key(|k| key(corners[k].view));
        // 2. Cut walls at each record: into the cut set, or marked.
        let mut sets: BTreeMap<[i32; 2], (u8, u8)> = BTreeMap::new();
        for k in &order {
            let c = &corners[k];
            let bits = |i: usize| {
                let d = walls[i].other(c.view);
                bit([d[0] - c.view[0], d[1] - c.view[1]])
            };
            let mut standing = c.slots.iter().flatten().fold(0, |m, &i| m | bits(i));
            let mut cut = 0;
            let active = c.slots.map(|s| s.map(|i| flags[i] & 1 != 0));
            for (s, wall) in c.slots.iter().enumerate() {
                let Some(i) = *wall else { continue };
                if active[s] != Some(true) {
                    continue;
                }
                let alone = (0..4).all(|o| {
                    o == s
                        || match (c.slots[o], active[o]) {
                            (Some(j), Some(false)) => !solid(walls[j].style),
                            _ => true,
                        }
                });
                let b = bits(i);
                if alone {
                    if standing & b != 0 {
                        cut |= b;
                        standing &= !b;
                    }
                } else if flags[i] & 6 == 0 {
                    flags[i] |= if s == 0 || s == 3 { 2 } else { 4 };
                }
            }
            sets.insert(*k, (standing, cut));
        }
        // 3. The masks, the trim and the cap.
        for k in &order {
            let (standing, cut) = sets[k];
            let c = corners.get_mut(k).unwrap();
            let (mut excluded, mut marked) = (0u8, 0u8);
            for &i in c.slots.iter().flatten() {
                let d = walls[i].other(c.view);
                let b = bit([d[0] - c.view[0], d[1] - c.view[1]]);
                let (f, style) = (flags[i], walls[i].style);
                if f & 6 == 0 {
                    if matches!(style, 2 | 12 | 13 | 14) {
                        excluded |= b;
                    }
                } else {
                    match walls[i].run().0 {
                        Line::Y | Line::X => marked |= b,
                        Line::Falling if f & 2 != 0 => marked |= b,
                        Line::Falling => excluded |= b,
                        Line::Rising => {}
                    }
                }
                if f & 1 != 0 && door_like(style) {
                    excluded |= b;
                }
            }
            c.standing = standing & !excluded & !marked;
            c.cut = cut & !excluded & !marked;
            // The cap's shape (0x53c70c): cut when the corner has a cut
            // cap at all, unless the cap's wall is cut and marked; then cut
            // only if the mark is at the wall's other end.
            let low = cap(cut, excluded, 0);
            let shape = |slot: usize| match c.slots[slot].map(|i| flags[i]) {
                Some(f) if f & 1 != 0 && f & 6 != 0 => {
                    f & if slot == 0 || slot == 3 { 4 } else { 2 } != 0
                }
                _ => low.is_some(),
            };
            c.cap = cap(standing, excluded, marked)
                .or(low)
                .map(|cap| (cap, shape(cap.slot)));
        }
        Corners {
            walls,
            flags,
            corners,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(a: [i32; 2], b: [i32; 2], cut: bool) -> Wall {
        Wall {
            lot: [a, b],
            view: [a, b],
            style: 1,
            cut,
        }
    }

    #[test]
    fn records_stand_where_walls_end_or_turn() {
        assert!(record(1));
        assert!(record(1 | 2));
        assert!(!record(1 | 4));
        assert!(!record(0x10 | 0x40));
        assert!(record(1 | 4 | 8));
        // A diagonal crossing a straight wall's corner is no record.
        assert!(!record(0x10 | 1));
    }

    #[test]
    fn cap_descriptors_follow_the_original_table() {
        let c = cap(1, 0, 0).unwrap();
        assert_eq!(
            (c.offset, c.width, c.slot, c.line, c.right),
            ([-1, 0], 3, 0, Line::X, false)
        );
        let c = cap(6, 0, 0).unwrap();
        assert_eq!(
            (c.offset, c.slot, c.line, c.right),
            ([0, -1], 2, Line::Y, true)
        );
        assert_eq!(cap(0x90, 0, 0).unwrap().line, Line::Falling);
        // Straight through, or an L opening toward the viewer: no cap.
        assert_eq!(cap(5, 0, 0), None);
        assert_eq!(cap(3, 0, 0), None);
        // Excluded walls drop out first; included ones join after.
        assert_eq!(cap(4, 4, 0), None);
        assert_eq!(cap(4, 0, 2).unwrap().slot, 2);
    }

    #[test]
    fn a_lone_standing_end_takes_a_cap() {
        let c = Corners::new(vec![wall([5, 5], [5, 6], false)]);
        let top = &c.corners[&[5, 5]];
        assert!(top.record);
        assert_eq!(top.standing, 4);
        let (cap, low) = top.cap.unwrap();
        assert_eq!((cap.slot, cap.line, low), (2, Line::Y, false));
        let bottom = &c.corners[&[5, 6]];
        assert_eq!(bottom.cap.unwrap().0.line, Line::X);
    }

    #[test]
    fn a_cut_wall_meeting_a_standing_one_is_marked() {
        // Standing along +x from (5, 5), cut along +y.
        let c = Corners::new(vec![
            wall([5, 5], [6, 5], false),
            wall([5, 5], [5, 6], true),
        ]);
        let corner = &c.corners[&[5, 5]];
        // Slot 2 marks with bit 4.
        assert_eq!(c.flags[1], 1 | 4);
        assert_eq!((corner.standing, corner.cut), (8, 0));
        // 4 | 8 opens toward the viewer: no cap.
        assert_eq!(corner.cap, None);
        // The cut wall's free end is its own cap, at the cut height.
        let end = &c.corners[&[5, 6]];
        assert_eq!(end.cut, 0);
        let (cap, low) = end.cap.unwrap();
        assert_eq!((cap.slot, low), (0, true));
    }

    #[test]
    fn a_cut_run_next_to_a_standing_one_is_marked_along_the_run() {
        // One run along +y through (5, 6): standing, then cut.
        let c = Corners::new(vec![
            wall([5, 5], [5, 6], false),
            wall([5, 6], [5, 7], true),
        ]);
        assert!(!c.corners[&[5, 6]].record);
        // The cut segment's near end.
        assert_eq!(c.flags[1], 1 | 4);
        // Its far end is a record with no standing wall: the cut set,
        // but marked walls take no trim.
        let end = &c.corners[&[5, 7]];
        assert_eq!((end.standing, end.cut), (0, 0));
    }
}
