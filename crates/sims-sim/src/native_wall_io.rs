//! Original4795c5/4797e2/479c58 over borrowed retained matrix owners.
//! Planning qualifies the consumed input before a non-yielding rotation0 call;
//! it is not a native error/rollback path. The small ordered store tape and its
//! read witnesses can then resume cold without copying/replacing a world.
use crate::{
    saved_matrix::{Allocation, Matrix, Stop},
    saved_world,
    surface_remap::{Coord, ExpandedWall, ObservedByte},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn profile(active: i32, at: Coord) -> Result<(), Stop> {
    if !(0..=127).contains(&active) || at.x > 127 || at.y > 127 || !(1..=2).contains(&at.level) {
        return Err(Stop::Unsupported(
            "wall I/O outside signed-coordinate profile".into(),
        ));
    }
    Ok(())
}
fn raw(matrix: &Matrix, x: usize, y: usize) -> Result<&[u8], Stop> {
    saved_world::cell(matrix, x, y, 14)
}
fn known_word(wall: &ExpandedWall, offset: usize) -> Result<u16, Stop> {
    Ok(u16::from_le_bytes([
        known_bits(wall, offset, 255)?,
        known_bits(wall, offset + 1, 255)?,
    ]))
}
fn known_bits(wall: &ExpandedWall, offset: usize, mask: u8) -> Result<u8, Stop> {
    if wall.0.len() != 44 {
        return Err(Stop::Unsupported(
            "expanded wall needs44 byte positions".into(),
        ));
    }
    let byte = wall
        .0
        .get(offset)
        .ok_or_else(|| Stop::Unsupported("expanded field outside wall".into()))?;
    if byte.known & mask != mask {
        return Err(Stop::Unavailable(
            "consumed expanded wall bits unavailable".into(),
        ));
    }
    Ok(byte.value & mask)
}

/// The selected level's actual14-byte matrix, with independent column offsets.
/// East/south ownership and adjacency use active, not allocated dimensions.
pub fn get(walls: &Matrix, active: i32, at: Coord) -> Result<ExpandedWall, Stop> {
    get_with(active, at, |x, y| Ok(raw(walls, x, y)?.try_into().unwrap()))
}
/// Same getter over the presenter's live14-byte cells. Allocation and active
/// extents remain separate; only a small consumed cell is materialized.
pub fn get_cells(
    size: usize,
    active: usize,
    cells: &[sims_formats::lot::WallTile],
    at: Coord,
) -> Result<ExpandedWall, Stop> {
    if size > 127 || active > size || cells.len() != size * size {
        return Err(Stop::Unsupported(
            "expanded wall cell backing differs".into(),
        ));
    }
    get_with(active as i32, at, |x, y| {
        let c = if x < size && y < size {
            &cells[x * size + y]
        } else {
            return Err(Stop::Unavailable(
                "expanded wall cell outside retained backing".into(),
            ));
        };
        let mut b = [0; 14];
        b[0] = c.segments;
        b[1] = c.flags;
        for (n, v) in c.styles.iter().chain(c.patterns.iter()).enumerate() {
            b[2 + n * 2..4 + n * 2].copy_from_slice(&v.to_le_bytes());
        }
        Ok(b)
    })
}
fn get_with(
    active: i32,
    at: Coord,
    raw: impl Fn(usize, usize) -> Result<[u8; 14], Stop>,
) -> Result<ExpandedWall, Stop> {
    profile(active, at)?;
    let (x, y) = (usize::from(at.x), usize::from(at.y));
    let current = raw(x, y)?;
    let mut out = vec![ObservedByte::unknown(); 44];
    let mut put = |offset: usize, bytes: &[u8]| {
        for (target, &value) in out[offset..offset + bytes.len()].iter_mut().zip(bytes) {
            *target = value.into();
        }
    };
    put(0, &u32::from(current[0]).to_le_bytes());
    let mut flags = current[1];
    for (offset, source) in [
        (8, 2),
        (10, 4),
        (16, 4),
        (18, 4),
        (20, 6),
        (22, 8),
        (24, 12),
        (26, 10),
    ] {
        put(offset, &current[source..source + 2]);
    }
    for (bit, offset) in [(16, 28), (32, 32)] {
        put(
            offset,
            if current[0] & bit != 0 {
                &current[10..14]
            } else {
                &[0; 4]
            },
        );
    }
    put(36, &[0; 4]);
    let within = |n: u8| i32::from(n) < active - 1;
    let mut adjacent = 0;
    if at.x > 0 && raw(x - 1, y)?[0] & 48 != 0 {
        adjacent |= 4;
    }
    if at.y > 0 && raw(x, y - 1)?[0] & 48 != 0 {
        adjacent |= 8;
    }
    if within(at.y) {
        let south = raw(x, y + 1)?;
        if south[0] & 48 != 0 {
            adjacent |= 1;
        }
        if south[0] & 2 != 0 {
            put(14, &south[4..6]);
            flags = (flags & !0xc0) | ((south[1] & 12) << 4);
        }
    }
    if within(at.x) {
        let east = raw(x + 1, y)?;
        if east[0] & 48 != 0 {
            adjacent |= 2;
        }
        if east[0] & 1 != 0 {
            put(12, &east[2..4]);
            flags = (flags & !0x30) | ((east[1] & 3) << 4);
        }
    }
    put(4, &u32::from(flags).to_le_bytes());
    out[40] = ObservedByte {
        value: adjacent | 16,
        known: 31,
    };
    Ok(ExpandedWall(out))
}

/// Original479c58: outside the interior band, a source cell contributes zero.
pub fn vertex(walls: &Matrix, active: i32, at: Coord) -> Result<u8, Stop> {
    profile(active, at)?;
    vertex_with(active, i32::from(at.x), i32::from(at.y), |x, y| {
        Ok(raw(walls, x, y)?[0])
    })
}
fn vertex_with(
    active: i32,
    x: i32,
    y: i32,
    mut read: impl FnMut(usize, usize) -> Result<u8, Stop>,
) -> Result<u8, Stop> {
    let mut masks = [0; 4];
    for (slot, (xx, yy)) in masks
        .iter_mut()
        .zip([(x - 1, y - 1), (x, y - 1), (x - 1, y), (x, y)])
    {
        if xx >= 1 && yy >= 1 && xx < active - 1 && yy < active - 1 {
            *slot = read(xx as usize, yy as usize)?;
        }
    }
    Ok((u8::from(masks[0] & 8 != 0) * 2)
        | (u8::from(masks[0] & 32 != 0) * 16)
        | u8::from(masks[1] & 1 != 0)
        | (u8::from(masks[1] & 16 != 0) * 128)
        | (u8::from(masks[2] & 4 != 0) * 4)
        | (u8::from(masks[2] & 16 != 0) * 32)
        | (u8::from(masks[3] & 2 != 0) * 8)
        | (u8::from(masks[3] & 32 != 0) * 64))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Target {
    Walls,
    Vertices,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub target: Target,
    pub at: Coord,
    pub offset: usize,
    pub before: Vec<u8>,
    pub value: Vec<u8>,
    pub instruction: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Witness {
    target: Target,
    at: Coord,
    column: u32,
    bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Put {
    writes: Vec<Write>,
    next: usize,
    walls_owner: u64,
    vertices_owner: u64,
    witnesses: Vec<Witness>,
}
fn backing(matrix: &Matrix) -> Result<&crate::saved_matrix::Backing, Stop> {
    match &matrix.allocation {
        Allocation::Known(backing) => Ok(backing),
        Allocation::Unavailable => Err(Stop::Unavailable("wall I/O backing unavailable".into())),
        _ => Err(Stop::Unsupported(
            "wall I/O absent/released allocation profile".into(),
        )),
    }
}
impl Put {
    pub fn writes(&self) -> &[Write] {
        &self.writes
    }
    pub fn completed(&self) -> bool {
        self.next == self.writes.len()
    }
    /// Retain this cursor and both matrix owners together. A caller must not
    /// interleave other writes during the original non-yielding call. Witnesses
    /// fence changed/reused inputs rather than publishing a stale derived byte.
    pub fn step(
        &mut self,
        walls: &mut Matrix,
        vertices: &mut Matrix,
    ) -> Result<Option<Write>, Stop> {
        if self.completed() {
            return Ok(None);
        }
        if backing(walls)?.token != self.walls_owner
            || backing(vertices)?.token != self.vertices_owner
        {
            return Err(Stop::Unsupported(
                "wall I/O owner changed during ordered call".into(),
            ));
        }
        for witness in &self.witnesses {
            let matrix = match witness.target {
                Target::Walls => &*walls,
                Target::Vertices => &*vertices,
            };
            let b = backing(matrix)?;
            if b.column_offsets.get(usize::from(witness.at.x)) != Some(&witness.column)
                || saved_world::cell(
                    matrix,
                    usize::from(witness.at.x),
                    usize::from(witness.at.y),
                    witness.bytes.len(),
                )? != witness.bytes
            {
                return Err(Stop::Unsupported(
                    "wall I/O input changed during ordered call".into(),
                ));
            }
        }
        let write = self
            .writes
            .get(self.next)
            .ok_or_else(|| Stop::Unsupported("wall I/O cursor outside tape".into()))?
            .clone();
        let matrix = match write.target {
            Target::Walls => walls,
            Target::Vertices => vertices,
        };
        let stride = if write.target == Target::Walls { 14 } else { 1 };
        let cell = saved_world::cell_mut(
            matrix,
            usize::from(write.at.x),
            usize::from(write.at.y),
            stride,
        )?;
        let end = write
            .offset
            .checked_add(write.value.len())
            .ok_or_else(|| Stop::Unsupported("wall store extent overflow".into()))?;
        if cell.get(write.offset..end) != Some(write.before.as_slice()) {
            return Err(Stop::Unsupported(
                "wall I/O expected-old store differs".into(),
            ));
        }
        cell[write.offset..end].copy_from_slice(&write.value);
        for witness in &mut self.witnesses {
            if witness.target == write.target && witness.at == write.at {
                witness.bytes[write.offset..end].copy_from_slice(&write.value);
            }
        }
        self.next += 1;
        Ok(Some(write))
    }
}

struct Planner<'a> {
    walls: &'a Matrix,
    vertices: &'a Matrix,
    level: u8,
    cells: BTreeMap<(Target, usize, usize), Vec<u8>>,
    witnesses: Vec<Witness>,
    writes: Vec<Write>,
}
impl Planner<'_> {
    fn load(&mut self, target: Target, x: usize, y: usize) -> Result<&mut Vec<u8>, Stop> {
        let key = (target, x, y);
        if !self.cells.contains_key(&key) {
            let matrix = if target == Target::Walls {
                self.walls
            } else {
                self.vertices
            };
            let stride = if target == Target::Walls { 14 } else { 1 };
            let bytes = saved_world::cell(matrix, x, y, stride)?.to_vec();
            let column = *backing(matrix)?
                .column_offsets
                .get(x)
                .ok_or_else(|| Stop::Unsupported("wall I/O column absent".into()))?;
            self.witnesses.push(Witness {
                target,
                at: Coord {
                    x: x.try_into()
                        .map_err(|_| Stop::Unsupported("wall X outside coordinate".into()))?,
                    y: y.try_into()
                        .map_err(|_| Stop::Unsupported("wall Y outside coordinate".into()))?,
                    level: self.level,
                },
                column,
                bytes: bytes.clone(),
            });
            self.cells.insert(key, bytes);
        }
        Ok(self.cells.get_mut(&key).unwrap())
    }
    fn byte(&mut self, x: usize, y: usize, offset: usize) -> Result<u8, Stop> {
        Ok(self.load(Target::Walls, x, y)?[offset])
    }
    fn emit(
        &mut self,
        target: Target,
        x: usize,
        y: usize,
        offset: usize,
        value: Vec<u8>,
        instruction: u32,
    ) -> Result<(), Stop> {
        let cell = self.load(target, x, y)?;
        let before = cell[offset..offset + value.len()].to_vec();
        cell[offset..offset + value.len()].copy_from_slice(&value);
        self.writes.push(Write {
            target,
            at: Coord {
                x: x as u8,
                y: y as u8,
                level: self.level,
            },
            offset,
            before,
            value,
            instruction,
        });
        Ok(())
    }
    fn b(&mut self, x: usize, y: usize, offset: usize, value: u8, ip: u32) -> Result<(), Stop> {
        self.emit(Target::Walls, x, y, offset, vec![value], ip)
    }
    fn w(&mut self, x: usize, y: usize, offset: usize, value: u16, ip: u32) -> Result<(), Stop> {
        self.emit(
            Target::Walls,
            x,
            y,
            offset,
            value.to_le_bytes().to_vec(),
            ip,
        )
    }
    fn change(
        &mut self,
        x: usize,
        y: usize,
        offset: usize,
        keep: u8,
        add: u8,
        ip: u32,
    ) -> Result<(), Stop> {
        let value = self.byte(x, y, offset)? & keep | add;
        self.b(x, y, offset, value, ip)
    }
}

/// Preflight is a pre-call source/profile gate, not native failure or rollback.
/// The overlay copies only the few touched/read cells, never the whole matrix.
pub fn plan_put(
    walls: &Matrix,
    vertices: &Matrix,
    active: i32,
    at: Coord,
    wall: &ExpandedWall,
) -> Result<Put, Stop> {
    profile(active, at)?;
    if at.x == 127 || at.y == 127 {
        return Err(Stop::Unsupported(
            "wall vertex exceeds signed-coordinate profile".into(),
        ));
    }
    let rotation = u32::from_le_bytes([
        known_bits(wall, 36, 255)?,
        known_bits(wall, 37, 255)?,
        known_bits(wall, 38, 255)?,
        known_bits(wall, 39, 255)?,
    ]);
    if rotation != 0 {
        return Err(Stop::Unsupported(
            "nonzero native wall rotation is unqualified".into(),
        ));
    }
    if backing(walls)?.token == backing(vertices)?.token {
        return Err(Stop::Unsupported(
            "wall and derived vertex owners alias in separate backing views".into(),
        ));
    }
    let mask = known_bits(wall, 0, 63)?;
    let (x, y) = (usize::from(at.x), usize::from(at.y));
    let mut p = Planner {
        walls,
        vertices,
        level: at.level,
        cells: BTreeMap::new(),
        witnesses: vec![],
        writes: vec![],
    };
    for (bit, style, patterns, ips) in [
        (
            16,
            4,
            [4, 5],
            [0x47984f, 0x47985f, 0x479871, 0x47987f, 0x47988a, 0x479892],
        ),
        (
            32,
            5,
            [6, 7],
            [0x4798a9, 0x4798b9, 0x4798cb, 0x4798d9, 0x4798e4, 0x4798ec],
        ),
    ] {
        if mask & bit != 0 {
            p.b(x, y, 0, bit, ips[0])?;
            p.w(x, y, 4, known_word(wall, 8 + style * 2)?, ips[1])?;
            p.w(x, y, 10, known_word(wall, 20 + patterns[0] * 2)?, ips[2])?;
            p.w(x, y, 12, known_word(wall, 20 + patterns[1] * 2)?, ips[3])?;
            p.w(x, y, 2, known_word(wall, 8)?, ips[4])?;
            p.w(x, y, 6, known_word(wall, 20)?, ips[5])?;
        } else {
            p.change(
                x,
                y,
                0,
                !bit,
                0,
                if bit == 16 { 0x479898 } else { 0x4798f2 },
            )?;
        }
    }
    if mask & 1 != 0 {
        p.change(x, y, 0, 255, 1, 0x4798ff)?;
        p.w(x, y, 2, known_word(wall, 8)?, 0x479913)?;
        p.w(x, y, 6, known_word(wall, 20)?, 0x479927)?;
        p.change(x, y, 1, 255, known_bits(wall, 4, 3)?, 0x479932)?;
        if x > 0 {
            p.change(x - 1, y, 0, 255, 4, 0x47994f)?;
        }
    } else if p.byte(x, y, 0)? & 1 != 0 {
        p.change(x, y, 0, !1, 0, 0x47995e)?;
        if x > 0 {
            p.change(x - 1, y, 0, !4, 0, 0x479978)?;
            p.w(x - 1, y, 12, 0, 0x47997b)?;
        }
    }
    if mask & 2 != 0 {
        p.change(x, y, 0, 255, 2, 0x47998a)?;
        p.w(x, y, 4, known_word(wall, 10)?, 0x47999e)?;
        p.w(x, y, 8, known_word(wall, 22)?, 0x4799b2)?;
        p.change(x, y, 1, 255, known_bits(wall, 4, 12)?, 0x4799c0)?;
        if y > 0 {
            p.change(x, y - 1, 0, 255, 8, 0x4799da)?;
        }
    } else if p.byte(x, y, 0)? & 2 != 0 {
        p.change(x, y, 0, !2, 0, 0x4799f1)?;
        if y > 0 {
            p.change(x, y - 1, 0, !8, 0, 0x479a06)?;
            p.w(x, y - 1, 10, 0, 0x479a0b)?;
        }
    }
    if mask & 4 != 0 {
        p.change(x, y, 0, 255, 4, 0x479a1f)?;
        p.w(x, y, 12, known_word(wall, 24)?, 0x479a2f)?;
        if i32::from(at.x) < active - 1 {
            p.change(x + 1, y, 0, 255, 1, 0x479a58)?;
            p.w(x + 1, y, 2, known_word(wall, 12)?, 0x479a66)?;
            p.change(x + 1, y, 1, 255, known_bits(wall, 4, 48)? >> 4, 0x479a75)?;
        }
    } else if p.byte(x, y, 0)? & 4 != 0 {
        p.change(x, y, 0, !4, 0, 0x479a82)?;
        if i32::from(at.x) < active - 1 {
            p.change(x + 1, y, 0, !1, 0, 0x479aa3)?;
            p.w(x + 1, y, 6, 0, 0x479aa6)?;
        }
    }
    if mask & 8 != 0 {
        p.change(x, y, 0, 255, 8, 0x479ab7)?;
        p.w(x, y, 10, known_word(wall, 26)?, 0x479ac6)?;
        if i32::from(at.y) < active - 1 {
            p.change(x, y + 1, 0, 255, 2, 0x479af5)?;
            p.w(x, y + 1, 4, known_word(wall, 14)?, 0x479b03)?;
            p.change(x, y + 1, 1, 255, known_bits(wall, 4, 192)? >> 4, 0x479b14)?;
        }
    } else if p.byte(x, y, 0)? & 8 != 0 {
        p.change(x, y, 0, !8, 0, 0x479b21)?;
        if i32::from(at.y) < active - 1 {
            p.change(x, y + 1, 0, !2, 0, 0x479b47)?;
            p.w(x, y + 1, 8, 0, 0x479b4a)?;
        }
    }
    for ((xx, yy), ip) in [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)]
        .into_iter()
        .zip([0x479b8d, 0x479bcd, 0x479c0b, 0x479c45])
    {
        let value = vertex_with(active, xx as i32, yy as i32, |a, b| p.byte(a, b, 0))?;
        p.emit(Target::Vertices, xx, yy, 0, vec![value], ip)?;
    }
    // The overlay owns separate logical cells. A shared/overlapping physical
    // cell profile needs a shared byte-addressed owner, rather than pretending
    // these independent views have native alias semantics.
    for (index, a) in p.witnesses.iter().enumerate() {
        let start_a = a.column as usize + usize::from(a.at.y) * a.bytes.len();
        let end_a = start_a + a.bytes.len();
        for b in &p.witnesses[index + 1..] {
            if a.target != b.target {
                continue;
            }
            let start_b = b.column as usize + usize::from(b.at.y) * b.bytes.len();
            let end_b = start_b + b.bytes.len();
            if start_a < end_b && start_b < end_a {
                return Err(Stop::Unsupported(
                    "aliased wall I/O cells need a shared backing profile".into(),
                ));
            }
        }
    }
    Ok(Put {
        writes: p.writes,
        next: 0,
        walls_owner: backing(walls)?.token,
        vertices_owner: backing(vertices)?.token,
        witnesses: p.witnesses,
    })
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
mod tests;
