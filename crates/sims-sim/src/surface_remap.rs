//! Original 4a0e3b/4c5d35 saved surface remapping, before saved types/ObjM.
//! Map codecs, catalog queries and 14-to-44-byte wall access remain host services.
use crate::{
    saved_matrix::{Matrix, Stop},
    saved_world,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Floor,
    Wall,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub saved: u16,
    pub local: u16,
    pub key: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entries {
    pub allocation: Option<u64>,
    pub used: usize,
    pub capacity: usize,
    /// Only consumed when building the table, not when retiring old groups.
    pub values: Option<Vec<Entry>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Original bytes before NUL; matching/encoding semantics belong to Host.
    /// Old names need not be available merely to destroy their entry vectors.
    pub name: Option<Vec<u8>>,
    pub entries: Entries,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Groups {
    pub allocation: Option<u64>,
    pub capacity: usize,
    pub values: Vec<Group>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub groups: Groups,
    pub runtime_files: Vec<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coord {
    pub x: u8,
    pub y: u8,
    pub level: u8,
}
/// Unknown bits remain symbolic; native OR16 defines just that dirty bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedByte {
    pub value: u8,
    pub known: u8,
}
impl ObservedByte {
    pub const fn unknown() -> Self {
        Self { value: 0, known: 0 }
    }
}
impl From<u8> for ObservedByte {
    fn from(value: u8) -> Self {
        Self {
            value,
            known: u8::MAX,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedWall(pub Vec<ObservedByte>);
impl ExpandedWall {
    pub fn known(bytes: impl IntoIterator<Item = u8>) -> Self {
        Self(bytes.into_iter().map(ObservedByte::from).collect())
    }
    fn valid(&self) -> Result<(), Stop> {
        if self.0.len() != 44 {
            return Err(Stop::Unsupported(
                "expanded wall must have 44 byte positions".into(),
            ));
        }
        Ok(())
    }
    fn bytes(&self, at: usize, count: usize) -> Result<Vec<u8>, Stop> {
        self.valid()?;
        self.0
            .get(at..at + count)
            .ok_or_else(|| Stop::Unsupported("expanded field outside wall".into()))?
            .iter()
            .map(|b| (b.known == u8::MAX).then_some(b.value))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| Stop::Unavailable("consumed expanded wall bytes unavailable".into()))
    }
    fn word(&self, at: usize) -> Result<u16, Stop> {
        Ok(u16::from_le_bytes(self.bytes(at, 2)?.try_into().unwrap()))
    }
    fn mask(&self) -> Result<u32, Stop> {
        Ok(u32::from_le_bytes(self.bytes(0, 4)?.try_into().unwrap()))
    }
    fn set(&mut self, at: usize, value: u16) -> Result<(), Stop> {
        self.word(at)?;
        self.0[at..at + 2].copy_from_slice(&value.to_le_bytes().map(ObservedByte::from));
        // Native OR DWORD [wall+40],16 preserves every other bit. Unknown
        // upper bits are neither read as fabricated input nor overwritten.
        self.0[40].value |= 16;
        self.0[40].known |= 16;
        Ok(())
    }
}

pub trait Host {
    /// Actual allocator service. A stop after side effects must retain its
    /// pending reply/partial cursor, and must not repeat them on retry.
    fn free_vector(&mut self, owner: u64, bytes: usize) -> Result<(), Stop>;
    /// May publish partial decoded groups before a native error or host stop.
    fn read_map(&mut self, kind: Kind, id: u32, target: &mut Groups) -> Result<i32, Stop>;
    /// Explicit codec/append boundary: legacy byte IDs widen. Complete source/
    /// vector behavior must be supplied by the host, not inferred from this leaf.
    fn normalize_legacy(&mut self, source: &Groups, target: &mut Groups) -> Result<(), Stop>;
    fn allocate_table(&mut self, bytes: usize) -> Result<u64, Stop>;
    fn free_table(&mut self, owner: u64) -> Result<(), Stop>;
    fn match_file(&mut self, name: &[u8], file: u64) -> Result<bool, Stop>;
    fn resolve_direct(&mut self, kind: Kind, index: usize, local: u16) -> Result<u16, Stop>;
    fn resolve_nearest(&mut self, kind: Kind, key: u32) -> Result<u16, Stop>;
    fn get_wall(&mut self, coord: Coord) -> Result<ExpandedWall, Stop>;
    fn put_wall(&mut self, coord: Coord, wall: &ExpandedWall) -> Result<(), Stop>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    BeginErase,
    Erase,
    LegacyClear,
    Read,
    Normalize,
    BeginDestroy,
    Destroy,
    DestroyStorage,
    ReadDone,
    Allocate,
    Group,
    Match,
    Entry,
    Direct,
    Nearest,
    Cell,
    GetWall,
    WallReady,
    FloorHalf,
    Pattern,
    PutWall,
    Free,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    kind: Kind,
    legacy: bool,
    phase: Phase,
    erase: usize,
    pub temporary: Groups,
    read_status: Option<i32>,
    pub table_owner: Option<u64>,
    pub table: Option<Vec<u16>>,
    group: usize,
    file: usize,
    found: usize,
    entry: usize,
    coord: Coord,
    half: usize,
    bit: u32,
    side: usize,
    changed: bool,
    pub wall: Option<ExpandedWall>,
    status: Option<i32>,
}
impl Execution {
    pub fn new(kind: Kind, version: i32) -> Self {
        Self {
            kind,
            legacy: version < 63,
            phase: Phase::BeginErase,
            erase: 0,
            temporary: Groups::default(),
            read_status: None,
            table_owner: None,
            table: None,
            group: 0,
            file: 0,
            found: 0,
            entry: 0,
            coord: Coord {
                x: 0,
                y: 0,
                level: 1,
            },
            half: 0,
            bit: 0,
            side: 0,
            changed: false,
            wall: None,
            status: None,
        }
    }
    pub fn status(&self) -> Option<i32> {
        self.status
    }
    /// One service or local transition. Checkpoint this cursor, State, both
    /// floor matrices and the actual wall/source/allocator Host together.
    /// No VM dispatch, RNG, primary-room rebuild or live tile publication.
    pub fn step(
        &mut self,
        state: &mut State,
        active: i32,
        mut floors: [&mut Matrix; 2],
        host: &mut impl Host,
    ) -> Result<Option<i32>, Stop> {
        match self.phase {
            Phase::BeginErase => {
                self.phase = Phase::Erase;
            }
            Phase::Erase => {
                if self.erase < state.groups.values.len() {
                    free_entries(&state.groups.values[self.erase].entries, host)?;
                    self.erase += 1;
                } else {
                    state.groups.values.clear();
                    self.erase = 0;
                    self.phase = if self.legacy {
                        Phase::LegacyClear
                    } else {
                        Phase::Read
                    };
                }
            }
            Phase::LegacyClear => {
                self.temporary.values.clear();
                self.phase = Phase::Read;
            }
            Phase::Read => {
                let target = if self.legacy {
                    &mut self.temporary
                } else {
                    &mut state.groups
                };
                self.read_status =
                    Some(host.read_map(self.kind, if self.legacy { 0 } else { 1 }, target)?);
                self.phase = if self.legacy {
                    if self.read_status == Some(0) {
                        Phase::Normalize
                    } else {
                        Phase::BeginDestroy
                    }
                } else {
                    Phase::ReadDone
                };
            }
            Phase::Normalize => {
                host.normalize_legacy(&self.temporary, &mut state.groups)?;
                self.phase = Phase::BeginDestroy;
            }
            Phase::BeginDestroy => {
                self.phase = Phase::Destroy;
            }
            Phase::Destroy => {
                if self.erase < self.temporary.values.len() {
                    free_entries(&self.temporary.values[self.erase].entries, host)?;
                    self.erase += 1;
                } else {
                    self.phase = Phase::DestroyStorage;
                }
            }
            Phase::DestroyStorage => {
                if self.temporary.capacity != 0
                    && let Some(owner) = self.temporary.allocation
                {
                    host.free_vector(owner, vector_bytes(self.temporary.capacity, 284)?)?;
                }
                // The local native descriptor is dead, rather than a new retained owner.
                self.temporary = Groups::default();
                self.phase = Phase::ReadDone;
            }
            Phase::ReadDone => {
                let status = self
                    .read_status
                    .ok_or_else(|| Stop::Unavailable("map status absent".into()))?;
                if status != 0 {
                    self.status = Some(status);
                    self.phase = Phase::Complete;
                } else {
                    self.phase = Phase::Allocate;
                }
            }
            Phase::Allocate => {
                let owner = host.allocate_table(65535 * 2)?;
                if owner == 0 {
                    return Err(Stop::Unsupported(
                        "null throwing surface table allocation profile".into(),
                    ));
                }
                self.table_owner = Some(owner);
                self.table = Some(vec![u16::MAX; 65535]);
                self.phase = Phase::Group;
            }
            Phase::Group => {
                if self.group >= state.groups.values.len() {
                    self.phase = Phase::Cell;
                } else {
                    self.file = 0;
                    self.found = 0;
                    self.entry = 0;
                    self.phase = Phase::Match;
                }
            }
            Phase::Match => {
                if self.file >= state.runtime_files.len() {
                    self.phase = Phase::Entry;
                } else {
                    let name = state.groups.values[self.group]
                        .name
                        .as_deref()
                        .ok_or_else(|| Stop::Unavailable("saved group name unavailable".into()))?;
                    if host.match_file(name, state.runtime_files[self.file])? {
                        self.found = self.file;
                        self.phase = Phase::Entry;
                    } else {
                        self.file += 1;
                    }
                }
            }
            Phase::Entry => {
                let entries = &state.groups.values[self.group].entries;
                if self.entry >= entries.used {
                    self.group += 1;
                    self.phase = Phase::Group;
                } else {
                    let entry = self.current_entry(state)?;
                    self.table_value(entry.saved)?;
                    self.phase = if self.found != 0 {
                        Phase::Direct
                    } else {
                        Phase::Nearest
                    };
                }
            }
            Phase::Direct => {
                let entry = self.current_entry(state)?.clone();
                let value = host.resolve_direct(self.kind, self.found, entry.local)?;
                self.set_table(entry.saved, value)?;
                self.phase = Phase::Nearest;
            }
            Phase::Nearest => {
                let entry = self.current_entry(state)?.clone();
                if self.table_value(entry.saved)? == u16::MAX {
                    let value = host.resolve_nearest(self.kind, entry.key)?;
                    self.set_table(entry.saved, if value == u16::MAX { 1 } else { value })?;
                }
                self.entry += 1;
                self.phase = Phase::Entry;
            }
            Phase::Cell => {
                let extent = usize::try_from(active.max(0))
                    .map_err(|_| Stop::Unsupported("active extent outside host".into()))?;
                if extent > 127 {
                    return Err(Stop::Unsupported(
                        "surface coordinate exceeds signed-byte profile".into(),
                    ));
                }
                if self.coord.level > 2 {
                    self.phase = Phase::Free;
                } else if usize::from(self.coord.y) >= extent {
                    self.coord.level += 1;
                    self.coord.x = 0;
                    self.coord.y = 0;
                } else if usize::from(self.coord.x) >= extent {
                    self.coord.x = 0;
                    self.coord.y += 1;
                } else if self.kind == Kind::Floor {
                    let m = &mut floors[usize::from(self.coord.level - 1)];
                    let x = usize::from(self.coord.x);
                    let y = usize::from(self.coord.y);
                    let old =
                        u16::from_le_bytes(saved_world::cell(m, x, y, 2)?.try_into().unwrap());
                    if eligible(old, self.kind) {
                        let new = self.table_value(old)?;
                        if old != new {
                            saved_world::cell_mut(m, x, y, 2)?.copy_from_slice(&new.to_le_bytes());
                        }
                        self.advance();
                    } else if old == 255 {
                        self.phase = Phase::GetWall;
                    } else {
                        self.advance();
                    }
                } else {
                    self.phase = Phase::GetWall;
                }
            }
            Phase::GetWall => {
                self.wall = Some(host.get_wall(self.coord)?);
                self.phase = Phase::WallReady;
            }
            Phase::WallReady => {
                let wall = self
                    .wall
                    .as_ref()
                    .ok_or_else(|| Stop::Unavailable("accepted wall reply absent".into()))?;
                wall.valid()?;
                let mask = wall.mask()?;
                self.bit = mask & mask.wrapping_neg();
                self.changed = false;
                self.half = 0;
                self.side = 0;
                self.phase = if self.kind == Kind::Floor {
                    if mask & 0x30 != 0 && matches!(self.bit, 0x10 | 0x20) {
                        Phase::FloorHalf
                    } else {
                        self.advance();
                        Phase::Cell
                    }
                } else if self.bit != 0 {
                    Phase::Pattern
                } else {
                    self.advance();
                    Phase::Cell
                };
            }
            Phase::FloorHalf => {
                if self.half >= 2 {
                    self.phase = Phase::PutWall;
                } else {
                    let offset = [8, 20][self.half];
                    self.remap_wall_field(offset)?;
                    self.half += 1;
                }
            }
            Phase::Pattern => {
                let count = if matches!(self.bit, 0x10 | 0x20) {
                    2
                } else {
                    1
                };
                if self.side >= count {
                    let mask = self
                        .wall
                        .as_ref()
                        .ok_or_else(|| Stop::Unavailable("wall local absent".into()))?
                        .mask()?;
                    let mut bit = self.bit.checked_mul(2).ok_or_else(|| {
                        Stop::Unsupported("wall segment iterator overflow".into())
                    })?;
                    while bit < 0x20 && mask & bit != bit {
                        bit *= 2;
                    }
                    if bit >= 0x20 {
                        self.phase = Phase::PutWall;
                    } else {
                        self.bit = bit;
                        self.side = 0;
                    }
                } else {
                    let side = self.pattern_side();
                    let offset = pattern_offset(self.bit, side)?;
                    self.remap_wall_field(offset)?;
                    self.side += 1;
                }
            }
            Phase::PutWall => {
                if self.changed {
                    host.put_wall(
                        self.coord,
                        self.wall
                            .as_ref()
                            .ok_or_else(|| Stop::Unavailable("wall local absent".into()))?,
                    )?;
                }
                self.advance();
                self.phase = Phase::Cell;
            }
            Phase::Free => {
                let owner = self
                    .table_owner
                    .ok_or_else(|| Stop::Unavailable("surface table allocation absent".into()))?;
                host.free_table(owner)?;
                self.table_owner = None;
                self.table = None;
                self.status = Some(0);
                self.phase = Phase::Complete;
            }
            Phase::Complete => {}
        }
        Ok(self.status)
    }
    fn advance(&mut self) {
        self.coord.x += 1;
        self.wall = None;
        self.changed = false;
    }
    fn pattern_side(&self) -> u32 {
        match self.bit {
            0x10 => [2, 4][self.side],
            0x20 => [1, 3][self.side],
            _ => 0,
        }
    }
    fn current_entry<'a>(&self, state: &'a State) -> Result<&'a Entry, Stop> {
        let entries = &state.groups.values[self.group].entries;
        if entries.used > entries.capacity {
            return Err(Stop::Unsupported("saved entries exceed capacity".into()));
        }
        entries
            .values
            .as_ref()
            .and_then(|v| v.get(self.entry))
            .ok_or_else(|| Stop::Unavailable("consumed saved mapping entry unavailable".into()))
    }
    fn table_value(&self, id: u16) -> Result<u16, Stop> {
        self.table
            .as_ref()
            .and_then(|t| t.get(usize::from(id)))
            .copied()
            .ok_or_else(|| Stop::Unsupported("surface ID outside 65535-word native table".into()))
    }
    fn set_table(&mut self, id: u16, value: u16) -> Result<(), Stop> {
        self.table_value(id)?;
        self.table.as_mut().unwrap()[usize::from(id)] = value;
        Ok(())
    }
    fn remap_wall_field(&mut self, offset: usize) -> Result<(), Stop> {
        let old = self
            .wall
            .as_ref()
            .ok_or_else(|| Stop::Unavailable("wall local absent".into()))?
            .word(offset)?;
        if eligible(old, self.kind) {
            let value = self.table_value(old)?;
            if old != value {
                self.wall.as_mut().unwrap().set(offset, value)?;
                self.changed = true;
            }
        }
        Ok(())
    }
}
fn eligible(id: u16, kind: Kind) -> bool {
    (match kind {
        Kind::Floor => (30..=254).contains(&id),
        Kind::Wall => (31..=247).contains(&id),
    }) || (256..=65534).contains(&id)
}
fn pattern_offset(bit: u32, side: u32) -> Result<usize, Stop> {
    if !bit.is_power_of_two() || bit > 0x20 {
        return Err(Stop::Unsupported("invalid native wall segment".into()));
    }
    let offset = 20
        + 2 * (bit.trailing_zeros() as usize
            + match side {
                1 | 4 => 1,
                3 => 2,
                _ => 0,
            });
    if offset + 2 > 40 {
        return Err(Stop::Unsupported(
            "wall pattern index outside expanded profile".into(),
        ));
    }
    Ok(offset)
}
fn vector_bytes(capacity: usize, stride: usize) -> Result<usize, Stop> {
    capacity
        .checked_mul(stride)
        .ok_or_else(|| Stop::Unsupported("surface vector byte extent overflow".into()))
}
fn free_entries(entries: &Entries, host: &mut impl Host) -> Result<(), Stop> {
    if entries.capacity != 0
        && let Some(owner) = entries.allocation
    {
        host.free_vector(owner, vector_bytes(entries.capacity, 8)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
