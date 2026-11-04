//! ObjectUndoable's embedded placement records and shared object-reference
//! catalogue. Object handles are stable lifetime identities, not script IDs.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub mod lifecycle;
pub mod live;
pub mod live_service;
pub mod placement;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub key: u32,
    pub position: [i32; 2],
    pub level: Option<i32>,
    pub facing: Option<i32>,
    pub container: Option<i16>,
    pub slot: Option<i32>,
    pub definition: Option<u32>,
    pub active: u8,
}
//48ba19 initializes key, XY and active only. The remaining fields must not
//be read as invented zeroes before a successful nonnull capture.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub script_id: i16,
    pub count: u32,
    pub key: u32,
    pub object: Option<u32>,
    pub removed: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct References {
    pub records: BTreeMap<u32, Reference>,
    pub order: Vec<u32>,
    pub next_key: u32,
    last_allocation: u32,
}
pub trait Registry {
    ///40743b. A missing script ID is a defined null object pointer.
    fn lookup(&self, script_id: i16) -> Result<Option<u32>, String>;
    ///432595, called only after unlinking the reference catalogue entry.
    fn on_lot(&mut self, object: u32) -> Result<bool, String>;
}
impl References {
    ///The owner supplies the original6c5fac value or restores saved state.
    pub fn new(next_key: u32) -> Self {
        Self {
            records: BTreeMap::new(),
            order: vec![],
            next_key,
            last_allocation: 0,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for &id in &self.order {
            if !seen.insert(id) || !self.records.contains_key(&id) {
                return Err("snapshot catalogue contains a duplicate or absent allocation".into());
            }
        }
        if self
            .records
            .iter()
            .any(|(&id, r)| id == 0 || id > self.last_allocation || r.object == Some(0))
        {
            return Err("snapshot catalogue has an invalid allocation or object identity".into());
        }
        // An unlinked allocation remains owned until its deletion callback
        // returns. Wrapping reference counts and duplicate keys are native.
        Ok(())
    }
    fn find(&self, predicate: impl Fn(&Reference) -> bool) -> Result<Option<u32>, String> {
        for &id in &self.order {
            let r = self
                .records
                .get(&id)
                .ok_or("snapshot catalogue allocation absent")?;
            if predicate(r) {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
    pub fn by_key(&self, key: u32) -> Result<Option<u32>, String> {
        if key == 0 {
            Ok(None)
        } else {
            self.find(|r| r.key == key)
        }
    }
    pub fn by_script_id(&self, id: i16) -> Result<Option<u32>, String> {
        if id == 0 {
            Ok(None)
        } else {
            self.find(|r| r.script_id == id)
        }
    }
    ///48c296: lookup by signed script ID, allocate if absent, increment count.
    pub fn acquire(&mut self, script_id: i16, view: &impl Registry) -> Result<u32, String> {
        if script_id == 0 {
            return Ok(0);
        }
        let allocation = if let Some(id) = self.by_script_id(script_id)? {
            id
        } else {
            let object = view.lookup(script_id)?;
            if object == Some(0) {
                return Err("null snapshot object must be represented as None".into());
            }
            let id = self
                .last_allocation
                .checked_add(1)
                .ok_or("snapshot allocation identities exhausted")?;
            self.records.insert(
                id,
                Reference {
                    script_id,
                    count: 0,
                    key: self.next_key,
                    object,
                    removed: 0,
                },
            );
            self.next_key = self.next_key.wrapping_add(1);
            self.order.push(id);
            self.last_allocation = id;
            id
        };
        let r = self.records.get_mut(&allocation).unwrap();
        r.count = r.count.wrapping_add(1);
        Ok(r.key)
    }
    ///48c17c's defined signed-word result. For key0 only AX is defined.
    pub fn script_id(&self, snapshot: &Snapshot) -> Result<i16, String> {
        Ok(self
            .by_key(snapshot.key)?
            .map(|id| self.records[&id].script_id)
            .unwrap_or(0))
    }
    ///48c144 gates on active/key, then requires a matching catalogue entry.
    pub fn object(&self, snapshot: &Snapshot) -> Result<Option<u32>, String> {
        if snapshot.active == 0 || snapshot.key == 0 {
            return Ok(None);
        }
        let id = self
            .by_key(snapshot.key)?
            .ok_or("active snapshot key has no reference record")?;
        Ok(self.records[&id].object)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delete {
    pub script_id: i16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum ReleasePhase {
    Entry,
    Inspect(u32),
    Delete {
        allocation: u32,
        request: Delete,
        acknowledged: bool,
    },
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    key: u32,
    phase: ReleasePhase,
}
impl Release {
    pub fn new(key: u32) -> Self {
        Self {
            key,
            phase: ReleasePhase::Entry,
        }
    }
    pub fn complete(&self) -> bool {
        self.phase == ReleasePhase::Complete
    }
    pub fn acknowledge(&mut self, request: Delete) -> Result<(), String> {
        match &mut self.phase {
            ReleasePhase::Delete {
                request: r,
                acknowledged,
                ..
            } if *r == request && !*acknowledged => {
                *acknowledged = true;
                Ok(())
            }
            _ => Err("snapshot deletion reply differs or has already returned".into()),
        }
    }
    pub fn advance(
        &mut self,
        refs: &mut References,
        view: &mut impl Registry,
    ) -> Result<Option<Delete>, String> {
        match self.phase {
            ReleasePhase::Complete => return Ok(None),
            ReleasePhase::Delete {
                allocation,
                request,
                acknowledged,
            } => {
                if !acknowledged {
                    return Ok(Some(request));
                }
                refs.records
                    .get_mut(&allocation)
                    .ok_or("snapshot allocation vanished during deletion")?
                    .object = None;
                refs.records.remove(&allocation);
                self.phase = ReleasePhase::Complete;
                return Ok(None);
            }
            ReleasePhase::Entry => (),
            ReleasePhase::Inspect(_) => (),
        }
        let id = if let ReleasePhase::Inspect(id) = self.phase {
            id
        } else {
            let Some(id) = refs.by_key(self.key)? else {
                self.phase = ReleasePhase::Complete;
                return Ok(None);
            };
            let r = refs.records.get_mut(&id).unwrap();
            r.count = r.count.wrapping_sub(1);
            if r.count != 0 {
                self.phase = ReleasePhase::Complete;
                return Ok(None);
            }
            //48c20e erases before checking the captured record's object. The
            //allocation survives so a callback cannot redirect cleanup by key.
            let index = refs
                .order
                .iter()
                .position(|&v| v == id)
                .ok_or("snapshot release lost its ordered record")?;
            refs.order.remove(index);
            self.phase = ReleasePhase::Inspect(id);
            id
        };
        let r = refs
            .records
            .get(&id)
            .ok_or("snapshot release allocation absent")?;
        if let Some(object) = r.object {
            if !view.on_lot(object)? {
                let request = Delete {
                    script_id: r.script_id,
                };
                self.phase = ReleasePhase::Delete {
                    allocation: id,
                    request,
                    acknowledged: false,
                };
                return Ok(Some(request));
            }
        }
        refs.records.remove(&id);
        self.phase = ReleasePhase::Complete;
        Ok(None)
    }
}

pub trait CaptureView: Registry {
    ///Non-dynamic multipart master (or self); None for ordinary/dynamic objects.
    fn normalized_master(&self, object: u32) -> Result<Option<u32>, String>;
    fn script_id(&self, object: u32) -> Result<i16, String>;
    ///43254a physical capture. It may normalize a zero-slot predecessor.
    fn capture(&mut self, object: u32) -> Result<Position, String>;
    fn definition(&self, object: u32) -> Result<u32, String>;
    fn facing(&self, object: u32) -> Result<i16, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub position: [i32; 2],
    pub level: i32,
    pub container: Option<u32>,
    pub slot: i16,
}
///48beb5. Zero input stores key0 but does not set active or clear old fields.
pub fn capture(
    snapshot: &mut Snapshot,
    refs: &mut References,
    mut script_id: i16,
    view: &mut impl CaptureView,
) -> Result<bool, String> {
    if snapshot.active != 0 {
        return Ok(false);
    }
    if script_id == 0 {
        snapshot.key = refs.acquire(0, view)?;
        return Ok(true);
    }
    let Some(mut object) = view.lookup(script_id)? else {
        return Ok(false);
    };
    if let Some(master) = view.normalized_master(object)? {
        object = master;
        script_id = view.script_id(object)?;
    }
    snapshot.key = refs.acquire(script_id, view)?;
    snapshot.active = 1;
    let d = view.capture(object)?;
    snapshot.position = d.position;
    snapshot.container = Some(match d.container {
        None => 0,
        Some(id) => view.script_id(id)?,
    });
    snapshot.slot = Some(i32::from(d.slot));
    snapshot.level = Some(d.level);
    snapshot.definition = Some(view.definition(object)?);
    snapshot.facing = Some(i32::from(view.facing(object)?));
    Ok(true)
}
