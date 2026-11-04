//! Native 43dd76/43dd2c/43a516 saved-type linking, before ObjM import.
//!
//! Source lookup, stream decoding and lazy function/signature ownership belong
//! to the supplied host. They must be checkpointed with this cursor and the
//! actual definition catalog; a missing service never becomes a default value.
//! This module does not replay neighborhood TATT or create saved objects.
use crate::object_catalog::{Key, definition::Catalog};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    pub name: Vec<u8>,
    pub guid: u32,
    /// Native declaration +10, zero before objt version 3.
    pub extra: u32,
    /// Sign-extended native declaration +14; -1 before version 2.
    pub kind: i32,
    pub saved_index: i16,
}

/// A decoded native record. The host consumes fields in source order: GUID,
/// signatures for version>0, index, kind for version>=2, padded byte string,
/// extra for version>=3. A zero GUID terminates before reading other fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub guid: u32,
    pub signatures: Option<[u32; 2]>,
    pub saved_index: i16,
    pub kind: Option<i16>,
    pub name: Vec<u8>,
    pub extra: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Function {
    Init,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gtab {
    Absent,
    /// The host retains the actual opened source owner. Present Gtab is an
    /// explicit unsupported continuation, never an identity-map success.
    Present,
}

pub trait Host {
    /// 43dd2c destroys each nonnull declaration in native vector order, then
    /// clears the vector. An error means this individual retirement is pending.
    fn retire_declaration(&mut self, declaration: &Declaration) -> Result<(), String>;
    fn gtab(&mut self) -> Result<Gtab, String>;
    /// Generic wrapper lookup/header read. None is an observed absent objt#0.
    fn begin_objt(&mut self) -> Result<Option<i32>, String>;
    fn next_objt(&mut self, version: i32) -> Result<Option<Record>, String>;
    /// Release the actual source handle only after the record reader returns.
    fn end_objt(&mut self) -> Result<(), String>;
    /// Perform the original lazy getter, including its owned OBJf/OBJD fallback
    /// and routine-cache effects. On error, preserve its actual pending state.
    fn signature(&mut self, resource: Key<u32>, function: Function) -> Result<u32, String>;
}

/// The catalog's distinct +a4 words, old declaration vector and +1270 lookup.
/// These are explicit supplied state, separate from registration ordinals and
/// from the immutable OBJD bytes. No unknown prior history is filled by serde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    keys: Vec<Key<u32>>,
    pub saved_indices: Vec<i16>,
    pub declarations: Vec<Option<Declaration>>,
    pub type_guids: Vec<u32>,
}
impl State {
    pub fn from_parts(
        catalog: &Catalog,
        saved_indices: Vec<i16>,
        declarations: Vec<Option<Declaration>>,
        type_guids: Vec<u32>,
    ) -> Result<Self, String> {
        if saved_indices.len() != catalog.entries.len() {
            return Err("saved-type index backing differs from catalog length".into());
        }
        Ok(Self {
            keys: catalog.entries.iter().map(|entry| entry.key).collect(),
            saved_indices,
            declarations,
            type_guids,
        })
    }
    pub fn validate(&self, catalog: &Catalog) -> Result<(), String> {
        if self.saved_indices.len() != self.keys.len()
            || !catalog
                .entries
                .iter()
                .map(|entry| entry.key)
                .eq(self.keys.iter().copied())
        {
            return Err("saved-type catalog ownership/order changed during read".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Reset,
    Retire(usize),
    Gtab,
    PresentGtab,
    Open,
    Next(i32),
    Link {
        version: i32,
        record: Record,
    },
    Init {
        version: i32,
        entry: usize,
        record: Record,
    },
    Main {
        version: i32,
        entry: usize,
        record: Record,
    },
    Release,
    ClearLookup,
    Publish(usize),
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    phase: Phase,
    /// Allocation/service limit, not a native rejection rule. Negative +a4
    /// sign-extends to a huge unsigned extent in 43dd76; never reinterpret it
    /// as a small u16 index or silently allocate gigabytes.
    lookup_capacity: usize,
}
impl Execution {
    pub fn new(lookup_capacity: usize) -> Self {
        Self {
            phase: Phase::Reset,
            lookup_capacity,
        }
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    /// One retained host boundary or local phase. Returns true at completion.
    /// Successful earlier writes survive a later error; retry does not repeat
    /// completed signature calls or declaration retirements.
    pub fn step(
        &mut self,
        state: &mut State,
        catalog: &mut Catalog,
        host: &mut impl Host,
    ) -> Result<bool, String> {
        state.validate(catalog)?;
        match self.phase.clone() {
            Phase::Reset => {
                state.saved_indices.fill(0);
                self.phase = Phase::Retire(0);
            }
            Phase::Retire(next) => {
                if next > state.declarations.len() {
                    return Err("saved-type declaration retirement cursor outside vector".into());
                }
                if let Some(value) = state.declarations.get(next) {
                    if let Some(declaration) = value {
                        host.retire_declaration(declaration)?;
                    }
                    self.phase = Phase::Retire(next + 1);
                } else {
                    state.declarations.clear();
                    self.phase = Phase::Gtab;
                }
            }
            Phase::Gtab => {
                self.phase = match host.gtab()? {
                    Gtab::Absent => Phase::Open,
                    Gtab::Present => Phase::PresentGtab,
                };
            }
            Phase::PresentGtab => {
                return Err(
                    "saved-type read requires native present-Gtab translation/owner services"
                        .into(),
                );
            }
            Phase::Open => {
                self.phase = host.begin_objt()?.map_or(Phase::ClearLookup, Phase::Next);
            }
            Phase::Next(version) => {
                let Some(record) = host.next_objt(version)? else {
                    self.phase = Phase::Release;
                    return Ok(false);
                };
                if record.guid == 0 {
                    self.phase = Phase::Release;
                    return Ok(false);
                }
                self.phase = Phase::Link { version, record };
            }
            Phase::Link { version, record } => {
                if record.signatures.is_some() != (version > 0)
                    || record.kind.is_some() != (version >= 2)
                    || record.extra.is_some() != (version >= 3)
                {
                    return Err(
                        "saved-type source record disagrees with native version field order".into(),
                    );
                }
                if let Some(&entry) = catalog.guids.get(&record.guid) {
                    if entry >= catalog.entries.len() {
                        return Err("saved-type GUID lookup points outside catalog".into());
                    }
                    state.saved_indices[entry] = record.saved_index;
                    if version <= 0 {
                        let data = &catalog.entries[entry].data;
                        let raw = data
                            .get(0x12..0x14)
                            .ok_or("saved-type legacy OBJD kind absent")?;
                        let changed = i16::from_le_bytes(raw.try_into().unwrap()) == 2;
                        set_changed(catalog, entry, changed);
                        self.phase = Phase::Next(version);
                    } else {
                        self.phase = Phase::Init {
                            version,
                            entry,
                            record,
                        };
                    }
                } else {
                    state.declarations.push(Some(Declaration {
                        guid: record.guid,
                        name: record.name,
                        saved_index: record.saved_index,
                        kind: record.kind.map_or(-1, i32::from),
                        extra: record.extra.unwrap_or(0),
                    }));
                    self.phase = Phase::Next(version);
                }
            }
            Phase::Init {
                version,
                entry,
                record,
            } => {
                let expected = record
                    .signatures
                    .ok_or("saved Init continuation lacks signatures")?;
                let key = catalog
                    .entries
                    .get(entry)
                    .ok_or("saved Init continuation resource absent")?
                    .key;
                let current = host.signature(key, Function::Init)?;
                if current != expected[0] {
                    set_changed(catalog, entry, true);
                    self.phase = Phase::Next(version);
                } else {
                    self.phase = Phase::Main {
                        version,
                        entry,
                        record,
                    };
                }
            }
            Phase::Main {
                version,
                entry,
                record,
            } => {
                let expected = record
                    .signatures
                    .ok_or("saved Main continuation lacks signatures")?;
                let key = catalog
                    .entries
                    .get(entry)
                    .ok_or("saved Main continuation resource absent")?
                    .key;
                let current = host.signature(key, Function::Main)?;
                set_changed(catalog, entry, current != expected[1]);
                self.phase = Phase::Next(version);
            }
            Phase::Release => {
                host.end_objt()?;
                self.phase = Phase::ClearLookup;
            }
            Phase::ClearLookup => {
                state.type_guids.clear();
                if self.lookup_capacity == 0 {
                    return Err("saved-type lookup requires capacity for its initial zero".into());
                }
                state.type_guids.push(0);
                self.phase = Phase::Publish(0);
            }
            Phase::Publish(next) => {
                if next > catalog.entries.len() {
                    return Err("saved-type publication cursor outside catalog".into());
                }
                if next == catalog.entries.len() {
                    self.phase = Phase::Complete;
                } else {
                    let index = (i32::from(state.saved_indices[next]) as u32) as usize;
                    let required = index
                        .checked_add(1)
                        .ok_or("native saved-type extent overflow")?;
                    if required > self.lookup_capacity {
                        return Err(format!(
                            "native saved-type lookup needs {required} slots; supplied capacity is {}",
                            self.lookup_capacity
                        ));
                    }
                    state
                        .type_guids
                        .resize(state.type_guids.len().max(required), 0);
                    let raw = catalog.entries[next]
                        .data
                        .get(28..32)
                        .ok_or("saved-type publication OBJD GUID absent")?;
                    state.type_guids[index] = u32::from_le_bytes(raw.try_into().unwrap());
                    self.phase = Phase::Publish(next + 1);
                }
            }
            Phase::Complete => {}
        }
        Ok(self.complete())
    }
}

fn set_changed(catalog: &mut Catalog, entry: usize, changed: bool) {
    let flags = &mut catalog.entries[entry].flags;
    *flags = (*flags & !2) | (u32::from(changed) << 1);
}
