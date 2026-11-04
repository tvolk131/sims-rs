//! Native478599 saved-world source arrays, before surfaces/types/ObjM.
//! Existing primary rooms and live tile-chain publication are separate owners.
use crate::saved_matrix::{self, Allocation, Matrix, Scratch, Stop};
use serde::{Deserialize, Serialize};

const ARRY: u32 = u32::from_be_bytes(*b"Arry");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lights {
    pub used: usize,
    pub capacity: usize,
    /// Known prefix of the allocation, possibly including an observed unused
    /// tail. Only used entries are consumed by reset; absent capacity tail is
    /// not a new native input requirement.
    pub values: Vec<[u8; 4]>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Floor {
    pub covering: Matrix,
    pub wall: Matrix,
    pub head: Matrix,
    pub flags: Matrix,
    /// Retained throughout this stage;478599 never reads/rebuilds this map.
    pub primary: Matrix,
    pub lights: Lights,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub active_size: i32,
    pub altitude: Matrix,
    pub ground: Matrix,
    pub grass: Matrix,
    pub target_grass: Matrix,
    pub pool: Matrix,
    pub water: Matrix,
    pub floors: [Floor; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Legacy {
    Floor,
    Wall,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Temporary {
    /// Object ownership is distinct from its matrix backing allocation token.
    pub owner: u64,
    pub matrix: Matrix,
}
pub trait Host: saved_matrix::Host {
    /// Actual64-square byte-floor/eight-byte-wall constructor, including its
    /// allocator/initialization effects. Missing services are not native errors.
    /// If a service stops after side effects, the host owns its pending reply/
    /// partial cursor and must not repeat completed effects on the next call.
    fn construct_temporary(&mut self, kind: Legacy) -> Result<Temporary, Stop>;
    /// Successful legacy copy destroys the matrix and its distinct object.
    /// Early native read errors bypass this ordinary cleanup.
    fn destroy_temporary(&mut self, temporary: &Temporary) -> Result<(), Stop>;
    ///478950 assigns diagnostic names, not new rooms or gameplay geometry.
    fn name_arrays(&mut self) -> Result<(), Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Slot {
    Altitude,
    Ground,
    Grass,
    TargetGrass,
    Covering(u8),
    Wall(u8),
    Head(u8),
    Flags(u8),
    Pool,
    Water,
    Temporary,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Operation {
    Read {
        slot: Slot,
        id: u32,
        converter: u32,
        ignore_status: bool,
    },
    Construct(Legacy),
    Copy {
        kind: Legacy,
        level: u8,
    },
    Destroy,
    Lights(u8),
    Names,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    version: i32,
    operations: Vec<Operation>,
    next: usize,
    read: Option<saved_matrix::Read>,
    pub temporary: Option<Temporary>,
    /// Native ignored failures may retain a source lease. Preserve that owner
    /// in research state; never release it by dropping a Rust local.
    pub abandoned_reads: Vec<saved_matrix::Read>,
    copy_x: usize,
    copy_y: usize,
    status: Option<i32>,
}
impl Execution {
    /// Complete Collection inputs use this version range. Older procedural
    /// grass/water initialization requires additional original semantics; this
    /// explicit host-profile gate is not a native version rejection/status.
    pub fn new(version: i32) -> Result<Self, Stop> {
        if version < 53 {
            return Err(Stop::Unsupported(
                "saved-world profile needs older grass/water services".into(),
            ));
        }
        let read = |slot, id, converter, ignore_status| Operation::Read {
            slot,
            id,
            converter,
            ignore_status,
        };
        let mut operations = vec![
            read(Slot::Altitude, 0, 0, false),
            read(Slot::Ground, 4, 0, false),
        ];
        for level in 0..2 {
            let base = u32::from(level) * 100;
            for kind in [Legacy::Floor, Legacy::Wall] {
                if version >= 63 {
                    let (slot, id) = match kind {
                        Legacy::Floor => (Slot::Covering(level), 11),
                        Legacy::Wall => (Slot::Wall(level), 12),
                    };
                    operations.push(read(slot, base + id, 0, false));
                } else {
                    operations.push(Operation::Construct(kind));
                    operations.push(read(
                        Slot::Temporary,
                        base + if kind == Legacy::Floor { 1 } else { 2 },
                        0,
                        false,
                    ));
                    operations.push(Operation::Copy { kind, level });
                    operations.push(Operation::Destroy);
                }
            }
            operations.push(read(
                Slot::Head(level),
                base + 3,
                saved_matrix::HEAD_CONVERTER,
                false,
            ));
            operations.push(read(Slot::Flags(level), base + 8, 0, false));
            operations.push(Operation::Lights(level));
        }
        operations.extend([
            read(Slot::Grass, 6, 0, false),
            read(Slot::TargetGrass, 7, 0, false),
            read(Slot::Pool, 9, 0, true),
            read(Slot::Water, 10, 0, true),
            Operation::Names,
        ]);
        Ok(Self {
            version,
            operations,
            next: 0,
            read: None,
            temporary: None,
            abandoned_reads: vec![],
            copy_x: 0,
            copy_y: 0,
            status: None,
        })
    }
    pub fn status(&self) -> Option<i32> {
        self.status
    }

    /// One source service or local transition. State, this cursor, Scratch and
    /// the actual source/allocator host must be retained together. No VM tick,
    /// RNG draw, primary-room rebuild or tile-head repair happens here.
    pub fn step(
        &mut self,
        state: &mut State,
        scratch: &mut Scratch,
        host: &mut impl Host,
    ) -> Result<Option<i32>, Stop> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        let operation = *self.operations.get(self.next).ok_or_else(|| {
            Stop::Unsupported("saved-world operation cursor outside sequence".into())
        })?;
        match operation {
            Operation::Read {
                slot,
                id,
                converter,
                ignore_status,
            } => {
                let matrix = match slot {
                    Slot::Temporary => {
                        &mut self
                            .temporary
                            .as_mut()
                            .ok_or_else(|| {
                                Stop::Unavailable("legacy temporary owner absent".into())
                            })?
                            .matrix
                    }
                    _ => state.matrix_mut(slot)?,
                };
                let read = self
                    .read
                    .get_or_insert_with(|| saved_matrix::Read::new(ARRY, id, converter));
                if let Some(status) = read.step(matrix, scratch, host)? {
                    if status != 0 && !ignore_status {
                        self.status = Some(status);
                        return Ok(self.status);
                    }
                    let completed = self.read.take().unwrap();
                    if completed.held_lease.is_some() {
                        self.abandoned_reads.push(completed);
                    }
                    self.next += 1;
                }
            }
            Operation::Construct(kind) => {
                if self.temporary.is_none() {
                    self.temporary = Some(host.construct_temporary(kind)?);
                }
                let temporary = self.temporary.as_ref().unwrap();
                if temporary.owner == 0 {
                    return Err(Stop::Unsupported(
                        "null legacy matrix-object constructor profile".into(),
                    ));
                }
                self.copy_x = 0;
                self.copy_y = 0;
                self.next += 1;
            }
            Operation::Copy { kind, level } => {
                let extent = usize::try_from(state.active_size.max(0))
                    .map_err(|_| Stop::Unsupported("active extent outside host".into()))?;
                if self.copy_y >= extent {
                    self.next += 1;
                } else if self.copy_x >= extent {
                    self.copy_x = 0;
                    self.copy_y += 1;
                } else {
                    let source = &self
                        .temporary
                        .as_ref()
                        .ok_or_else(|| Stop::Unavailable("legacy copy temporary absent".into()))?
                        .matrix;
                    let (source_stride, target_stride) = match kind {
                        Legacy::Floor => (1, 2),
                        Legacy::Wall => (8, 14),
                    };
                    let bytes = cell(source, self.copy_x, self.copy_y, source_stride)?;
                    let mut value = [0u8; 14];
                    match kind {
                        Legacy::Floor => {
                            value[..2].copy_from_slice(&u16::from(bytes[0]).to_le_bytes())
                        }
                        Legacy::Wall => {
                            value[..2].copy_from_slice(&bytes[..2]);
                            for i in 0..6 {
                                value[2 + i * 2..4 + i * 2]
                                    .copy_from_slice(&u16::from(bytes[2 + i]).to_le_bytes());
                            }
                        }
                    }
                    let slot = match kind {
                        Legacy::Floor => Slot::Covering(level),
                        Legacy::Wall => Slot::Wall(level),
                    };
                    cell_mut(
                        state.matrix_mut(slot)?,
                        self.copy_x,
                        self.copy_y,
                        target_stride,
                    )?
                    .copy_from_slice(&value[..target_stride]);
                    self.copy_x += 1;
                }
            }
            Operation::Destroy => {
                let temporary = self
                    .temporary
                    .as_ref()
                    .ok_or_else(|| Stop::Unavailable("legacy destructor owner absent".into()))?;
                host.destroy_temporary(temporary)?;
                self.temporary = None;
                self.next += 1;
            }
            Operation::Lights(level) => {
                let light = &mut state.floors[usize::from(level)].lights;
                if light.values.len() > light.capacity || light.used > light.capacity {
                    return Err(Stop::Unsupported(
                        "inconsistent retained light extents".into(),
                    ));
                }
                if light.values.len() < light.used {
                    return Err(Stop::Unavailable(
                        "used light allocation prefix is not retained".into(),
                    ));
                }
                saved_matrix::reset_lights(light.used, &mut light.values)?;
                self.next += 1;
            }
            Operation::Names => {
                host.name_arrays()?;
                self.next += 1;
                self.status = Some(0);
            }
        }
        Ok(self.status)
    }
}
impl State {
    fn matrix_mut(&mut self, slot: Slot) -> Result<&mut Matrix, Stop> {
        Ok(match slot {
            Slot::Altitude => &mut self.altitude,
            Slot::Ground => &mut self.ground,
            Slot::Grass => &mut self.grass,
            Slot::TargetGrass => &mut self.target_grass,
            Slot::Pool => &mut self.pool,
            Slot::Water => &mut self.water,
            Slot::Covering(level) => &mut self.floors[usize::from(level)].covering,
            Slot::Wall(level) => &mut self.floors[usize::from(level)].wall,
            Slot::Head(level) => &mut self.floors[usize::from(level)].head,
            Slot::Flags(level) => &mut self.floors[usize::from(level)].flags,
            Slot::Temporary => {
                return Err(Stop::Unsupported(
                    "temporary is not a world source slot".into(),
                ));
            }
        })
    }
}
fn span(
    matrix: &Matrix,
    x: usize,
    y: usize,
    stride: usize,
) -> Result<std::ops::Range<usize>, Stop> {
    let backing = match &matrix.allocation {
        Allocation::Known(backing) => backing,
        Allocation::Unavailable => {
            return Err(Stop::Unavailable("legacy copy backing unobserved".into()));
        }
        Allocation::Null | Allocation::Released { .. } => {
            return Err(Stop::Unsupported(
                "unsafe legacy copy through absent/released backing".into(),
            ));
        }
    };
    let offset = backing
        .column_offsets
        .get(x)
        .ok_or_else(|| Stop::Unsupported("legacy copy column outside retained directory".into()))?;
    let start = y
        .checked_mul(stride)
        .and_then(|n| n.checked_add(*offset as usize))
        .ok_or_else(|| Stop::Unsupported("legacy copy offset overflow".into()))?;
    let end = start
        .checked_add(stride)
        .ok_or_else(|| Stop::Unsupported("legacy copy extent overflow".into()))?;
    if end > backing.data.len() {
        return Err(Stop::Unsupported(
            "legacy copy outside retained allocation".into(),
        ));
    }
    Ok(start..end)
}
pub(crate) fn cell(matrix: &Matrix, x: usize, y: usize, stride: usize) -> Result<&[u8], Stop> {
    let range = span(matrix, x, y, stride)?;
    let Allocation::Known(backing) = &matrix.allocation else {
        unreachable!()
    };
    Ok(&backing.data[range])
}
pub(crate) fn cell_mut(
    matrix: &mut Matrix,
    x: usize,
    y: usize,
    stride: usize,
) -> Result<&mut [u8], Stop> {
    let range = span(matrix, x, y, stride)?;
    let Allocation::Known(backing) = &mut matrix.allocation else {
        unreachable!()
    };
    Ok(&mut backing.data[range])
}

#[cfg(test)]
mod tests;
