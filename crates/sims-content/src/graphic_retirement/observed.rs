//! Retained ordinary-Animator teardown. This is not a fresh graphics builder.
//! Native pointer translation and capture provenance belong to the importer.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::graphic_destruction::{Execution, Request, State, Storage, Vector};
use std::collections::{BTreeMap, BTreeSet};

pub type Id = u64;

pub mod frame;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    pub bytes: u32,
    pub live: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub bytes: Vec<u8>,
    pub allocation: Option<Id>,
    pub capacity: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Buffer {
    pub allocation: Option<Id>,
    pub capacity_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    pub bits: [u32; 16],
    pub flags: u32,
}

/// Named state preserved through Reset. All floats remain raw IEEE bits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetFields {
    pub scale_bits: [u32; 3],
    pub offset_bits: [u32; 3],
    pub dirty: [u8; 4],
    pub foreground_stamp: u32,
    pub engaged: u32,
    pub pending: Vec<u32>,
    pub pending_storage: Buffer,
    pub embedded_matrix: Matrix,
    /// Actual source constant671d64, not a caller-chosen reset default.
    pub invalid_scale_bits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bone {
    pub id: Id,
    pub name: Text,
    pub parent_name: Text,
    pub skin: Id,
    pub parent: Option<Id>,
    pub next_sibling: Option<Id>,
    pub local_pose_bits: [u32; 7],
    pub absolute_pose_bits: [u32; 7],
    pub flags: [u32; 3],
    pub wiggle_amplitude_bits: u32,
    pub wiggle_scale: u32,
    pub wiggle_records: [[u32; 4]; 4],
    pub named: u8,
    pub update: u32,
    pub geometry_stamp: u32,
}

/// The surviving skin remains owned after the model allocation is released.
/// Its old model identity is intentionally retained, including the dangling
/// native back-reference. No constructor or procedural RNG is run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skin {
    pub id: Id,
    pub references: u32,
    pub generation: u32,
    pub model_backpointer: Id,
    pub name: Text,
    pub parent_name: Text,
    pub pose: Matrix,
    pub root: Id,
    pub bones: Vec<Bone>,
    pub bones_storage: Buffer,
    pub lookup: Vec<Option<Id>>,
    pub lookup_storage: Buffer,
    pub layers: Vec<Id>,
    pub layers_storage: Buffer,
    pub appearances: Vec<Id>,
    pub appearances_storage: Buffer,
    pub hierarchy_dirty: u32,
    pub geometry_dirty: u32,
    pub hierarchy_evaluated: u32,
    pub hierarchy_generation: u32,
    pub bounds_generation: u32,
    pub vertex_generation: u32,
    pub pose_generation: u32,
    pub last_clock: u32,
    pub bounds_bits: [u32; 6],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: Id,
    pub skin: Id,
    pub notification: Id,
    pub resource_owner: Id,
    pub context_owner: Id,
    pub owner_identity: Id,
    pub name: Text,
    pub placement: Matrix,
    pub scale: Matrix,
    pub flags: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub id: Id,
    pub packed: u32,
    pub rectangle: [u32; 4],
    pub transform: Matrix,
    pub children: Vec<Id>,
    pub storage: Buffer,
    pub bounds_bits: [u32; 6],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: Id,
    pub binding: Option<Id>,
    pub name: Text,
    pub texture: Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retained {
    pub actor: Id,
    pub animator: Id,
    pub animator_name: Text,
    pub reset: ResetFields,
    pub layers: [Option<Id>; 3],
    pub model: Model,
    pub skin: Skin,
    pub notification: Notification,
    pub generic: Vec<Record>,
    pub generic_storage: Buffer,
    pub auxiliary: Vec<Record>,
    pub auxiliary_storage: Buffer,
    pub global_focus: Option<Id>,
    pub allocations: BTreeMap<Id, Allocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    Service(Request),
    SkinReference { id: Id, before: u32, after: u32 },
    NotificationReference { id: Id, before: u32, after: u32 },
    Free(Id),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    initial: Retained,
    retained: Retained,
    execution: Execution,
    state: State,
    events: Vec<Event>,
    outer_released: bool,
}

#[cfg(test)]
mod tests;

impl Owner {
    pub fn from_retained(retained: Retained) -> Result<Self> {
        ensure!(
            retained.allocations.values().all(|a| a.live),
            "retained input includes retired allocations"
        );
        let result = Self::new(retained);
        result.validate()?;
        Ok(result)
    }

    fn new(retained: Retained) -> Self {
        let state = State {
            model: Some(retained.model.id),
            layers: retained.layers,
            auxiliary: retained.auxiliary.iter().map(|r| Some(r.id)).collect(),
            generic: retained.generic.iter().map(|r| Some(r.id)).collect(),
            focus: retained.global_focus == Some(retained.actor),
        };
        Self {
            initial: retained.clone(),
            retained,
            execution: Execution::default(),
            state,
            events: vec![],
            outer_released: false,
        }
    }

    pub fn retained(&self) -> &Retained {
        &self.retained
    }
    pub fn events(&self) -> &[Event] {
        &self.events
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn service_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, Event::Service(_)))
            .count()
    }
    pub fn last_service(&self) -> Option<Request> {
        self.events.iter().rev().find_map(|e| {
            if let Event::Service(r) = e {
                Some(*r)
            } else {
                None
            }
        })
    }
    pub fn global_focus(&self) -> Option<Id> {
        self.retained.global_focus
    }
    pub fn complete(&self) -> bool {
        self.execution.complete()
    }
    pub fn outer_released(&self) -> bool {
        self.outer_released
    }

    pub fn validate(&self) -> Result<()> {
        self.structure()?;
        ensure!(
            self.events.len() <= 4096,
            "retained teardown history exceeds bound"
        );
        let mut replay = Self::new(self.initial.clone());
        replay.structure()?;
        ensure!(
            replay.retained.allocations.values().all(|a| a.live),
            "initial retained graph includes retired allocations"
        );
        for _ in self
            .events
            .iter()
            .filter(|e| matches!(e, Event::Service(_)))
        {
            ensure!(!replay.complete(), "service after completed teardown");
            replay.advance_owned()?;
        }
        if self.outer_released {
            replay.release_outer_owned()?;
        }
        ensure!(
            *self == replay,
            "retained teardown state differs from its owned service history"
        );
        Ok(())
    }

    fn structure(&self) -> Result<()> {
        self.execution
            .validate(&self.state)
            .map_err(anyhow::Error::msg)?;
        let r = &self.retained;
        ensure!(
            r.allocations.len() <= 65536
                && r.generic.len() <= 4096
                && r.auxiliary.len() <= 4096
                && r.skin.bones.len() <= 256
                && r.skin.lookup.len() <= 65536,
            "retained graphics owner bounds exceeded"
        );
        ensure!(
            r.actor != 0
                && r.animator != 0
                && r.model.id != 0
                && r.skin.id != 0
                && r.notification.id != 0,
            "missing retained graphics identity"
        );
        ensure!(
            r.model.skin == r.skin.id
                && r.skin.model_backpointer == r.model.id
                && r.model.notification == r.notification.id,
            "retained model/skin/notification aliases differ"
        );
        ensure!(
            r.reset.invalid_scale_bits == 0xc479c000,
            "unknown ordinary Reset constant"
        );
        ensure!(
            r.layers == [None; 3] && r.skin.layers.is_empty(),
            "nonempty retained layer teardown unsupported"
        );
        ensure!(
            r.skin.appearances.is_empty()
                && r.generic
                    .iter()
                    .chain(&r.auxiliary)
                    .all(|x| x.binding.is_none()),
            "retained appearance teardown requires live binding services"
        );
        ensure!(
            r.notification.children.is_empty(),
            "retained notification child teardown unsupported"
        );
        ensure!(
            self.state.model.is_none() || r.skin.references != 1,
            "retained last-reference skeletal destruction unsupported"
        );
        ensure!(
            r.skin
                .bones
                .iter()
                .all(|b| b.id != 0 && b.skin == r.skin.id)
                && r.skin.bones.iter().any(|b| b.id == r.skin.root),
            "retained bone owner differs"
        );
        let bone_ids: BTreeSet<_> = r.skin.bones.iter().map(|b| b.id).collect();
        ensure!(
            bone_ids.len() == r.skin.bones.len()
                && r.skin.lookup.iter().flatten().all(|i| bone_ids.contains(i)),
            "retained bone lookup alias differs"
        );
        let mut owned = BTreeSet::new();
        for (id, minimum) in [
            (r.animator, 0x138),
            (r.model.id, 0xb0),
            (r.skin.id, 0xd4),
            (r.notification.id, 0x80),
        ] {
            Self::allocation(r, &mut owned, id, minimum)?;
        }
        let text = |t: &Text, ids: &mut BTreeSet<Id>| -> Result<()> {
            ensure!(
                t.bytes.len() < t.capacity as usize || t.bytes.is_empty() && t.capacity == 0,
                "retained string bounds differ"
            );
            ensure!(!t.bytes.contains(&0), "retained string has embedded NUL");
            Self::buffer(r, ids, t.allocation, t.capacity)
        };
        let buffer = |b: &Buffer, ids: &mut BTreeSet<Id>| {
            Self::buffer(r, ids, b.allocation, b.capacity_bytes)
        };
        for t in [
            &r.animator_name,
            &r.model.name,
            &r.skin.name,
            &r.skin.parent_name,
        ] {
            text(t, &mut owned)?;
        }
        for b in [
            &r.reset.pending_storage,
            &r.generic_storage,
            &r.auxiliary_storage,
            &r.skin.bones_storage,
            &r.skin.lookup_storage,
            &r.skin.layers_storage,
            &r.skin.appearances_storage,
            &r.notification.storage,
        ] {
            buffer(b, &mut owned)?;
        }
        for b in &r.skin.bones {
            Self::allocation(r, &mut owned, b.id, 0x16c)?;
            text(&b.name, &mut owned)?;
            text(&b.parent_name, &mut owned)?;
        }
        for x in r.generic.iter().chain(&r.auxiliary) {
            Self::allocation(r, &mut owned, x.id, 0x1c)?;
            text(&x.name, &mut owned)?;
            text(&x.texture, &mut owned)?;
        }
        ensure!(
            owned.len() == r.allocations.len(),
            "unbound retained graphics allocation"
        );
        ensure!(
            r.reset.pending.len().saturating_mul(4)
                <= r.reset.pending_storage.capacity_bytes as usize
                && r.generic.len().saturating_mul(4) <= r.generic_storage.capacity_bytes as usize
                && r.auxiliary.len().saturating_mul(4)
                    <= r.auxiliary_storage.capacity_bytes as usize,
            "retained vector bounds differ"
        );
        for (count, b) in [
            (r.reset.pending.len(), &r.reset.pending_storage),
            (r.generic.len(), &r.generic_storage),
            (r.auxiliary.len(), &r.auxiliary_storage),
            (r.skin.bones.len(), &r.skin.bones_storage),
            (r.skin.lookup.len(), &r.skin.lookup_storage),
            (r.skin.layers.len(), &r.skin.layers_storage),
            (r.skin.appearances.len(), &r.skin.appearances_storage),
            (r.notification.children.len(), &r.notification.storage),
        ] {
            ensure!(
                b.capacity_bytes % 4 == 0 && count.saturating_mul(4) <= b.capacity_bytes as usize,
                "retained vector capacity differs"
            );
        }
        ensure!(
            self.state.model.is_none_or(|id| id == r.model.id),
            "retired model identity differs"
        );
        Ok(())
    }

    fn allocation(r: &Retained, owned: &mut BTreeSet<Id>, id: Id, minimum: u32) -> Result<()> {
        ensure!(
            id != 0 && owned.insert(id),
            "duplicate retained allocation owner"
        );
        ensure!(
            r.allocations
                .get(&id)
                .is_some_and(|a| a.bytes >= minimum && a.bytes <= 128 * 1024 * 1024),
            "retained allocation absent or undersized"
        );
        Ok(())
    }
    fn buffer(r: &Retained, ids: &mut BTreeSet<Id>, id: Option<Id>, capacity: u32) -> Result<()> {
        match (id, capacity) {
            (None, 0) => Ok(()),
            (Some(id), n) if n != 0 => Self::allocation(r, ids, id, n),
            _ => bail!("retained allocation/capacity owner differs"),
        }
    }
    fn free(&mut self, id: Id) -> Result<()> {
        let a = self
            .retained
            .allocations
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("release owner absent"))?;
        ensure!(a.live, "retained allocation released twice");
        a.live = false;
        self.events.push(Event::Free(id));
        Ok(())
    }
    fn free_optional(&mut self, id: Option<Id>) -> Result<()> {
        if let Some(id) = id {
            self.free(id)?;
        }
        Ok(())
    }

    /// Execute one original service with real owned mutations, then acknowledge
    /// the retained controller. A pending unsupported branch is never completed.
    pub fn advance(&mut self) -> Result<bool> {
        self.validate()?;
        let complete = self.advance_owned()?;
        self.validate()?;
        Ok(complete)
    }

    ///432387 releases the Animator allocation only after584898 returned.
    ///The caller still owns clearing object+120; it is not silently cleared here.
    pub fn release_animator(&mut self) -> Result<()> {
        self.validate()?;
        self.release_outer_owned()?;
        self.validate()
    }
    fn release_outer_owned(&mut self) -> Result<()> {
        ensure!(
            self.complete() && !self.outer_released,
            "premature or repeated outer Animator release"
        );
        self.free(self.retained.animator)?;
        self.outer_released = true;
        Ok(())
    }

    fn advance_owned(&mut self) -> Result<bool> {
        let Some(request) = self
            .execution
            .advance(&mut self.state)
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(self.execution.complete());
        };
        self.events.push(Event::Service(request));
        match request {
            Request::Reset => {
                let r = &mut self.retained;
                r.reset.scale_bits = [r.reset.invalid_scale_bits; 3];
                r.reset.offset_bits = [0; 3];
                r.reset.dirty = [1; 4];
                r.skin.generation = r.skin.generation.wrapping_add(1);
                r.reset.foreground_stamp = u32::MAX;
                r.reset.pending.clear();
                r.reset.engaged = 0;
            }
            Request::ClearAppearances => {
                ensure!(
                    self.retained
                        .generic
                        .iter()
                        .chain(&self.retained.auxiliary)
                        .all(|r| r.binding.is_none()),
                    "live appearance binding requires teardown"
                );
            }
            Request::DetachLayer(_) => bail!("nonempty retained layer teardown unsupported"),
            Request::DestroyModel(id) => {
                ensure!(id == self.retained.model.id, "stale model teardown");
                let before = self.retained.skin.references;
                let after = before.wrapping_sub(1);
                ensure!(after != 0, "last-reference skin teardown unsupported");
                self.retained.skin.references = after;
                self.events.push(Event::SkinReference {
                    id: self.retained.skin.id,
                    before,
                    after,
                });
                let before = self.retained.notification.packed;
                let arithmetic = ((before & 0xffffff00) | (before & 0xfc)).wrapping_sub(1);
                let after = ((arithmetic ^ before) & 0x3fffc) ^ before;
                self.retained.notification.packed = after;
                self.events.push(Event::NotificationReference {
                    id: self.retained.notification.id,
                    before,
                    after,
                });
                if after & 0x3fffc == 0 {
                    self.free_optional(self.retained.notification.storage.allocation)?;
                    self.free(self.retained.notification.id)?;
                }
                //5c38f0 is an actual RET for both embedded model matrices.
                self.free_optional(self.retained.model.name.allocation)?;
                self.free(id)?;
            }
            Request::ClearFocus => {
                ensure!(
                    self.retained.global_focus == Some(self.retained.actor),
                    "stale retained focus"
                );
                self.retained.global_focus = None;
            }
            Request::DestroyRecord {
                vector,
                index,
                record,
            } => {
                let list = match vector {
                    Vector::Auxiliary => &self.retained.auxiliary,
                    Vector::Generic => &self.retained.generic,
                };
                let x = list
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("retained record index absent"))?;
                ensure!(
                    x.id == record && x.binding.is_none(),
                    "retained record/binding differs"
                );
                let (name, texture) = (x.name.allocation, x.texture.allocation);
                self.free_optional(texture)?;
                self.free_optional(name)?;
                self.free(record)?;
            }
            Request::Storage(kind) => {
                let id = match kind {
                    Storage::Name => self.retained.animator_name.allocation,
                    Storage::Auxiliary => self.retained.auxiliary_storage.allocation,
                    Storage::Generic => self.retained.generic_storage.allocation,
                    Storage::Layers => self.retained.reset.pending_storage.allocation,
                };
                self.free_optional(id)?;
            }
        }
        self.execution
            .acknowledge(&mut self.state, request)
            .map_err(anyhow::Error::msg)?;
        Ok(false)
    }
}
