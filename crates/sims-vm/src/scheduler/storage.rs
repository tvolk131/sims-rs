//! Native registry vector ownership, separate from the linked enumeration list.
//! 4434cd appends one word for every pointer slot it grows, even when the word
//! vector was already longer, then initializes the selected word to0000ffff.
//! 44383c nulls the pointer without shrinking either vector. 44279a erases only
//! the pointer extent. Active words stay in Object::idle; vacant slots and the
//! separately retained word tail have no duplicate mutable live-word owner.
use crate::{Object, ObjectId, World, idle::IdleState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Slot {
    Vacant(u32),
    Occupied(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Storage {
    slots: Vec<Slot>,
    /// Replacement lifetime identity, not a native address or script ID.
    /// Never reused: a callback may replace an object at the same registry ID.
    last_handle: u32,
    /// Words after the pointer-vector end, not extra scheduler slots. Old wire
    /// payloads omit this field: their word backing remains explicitly unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    word_tail: Option<Vec<u32>>,
}
impl Default for Storage {
    fn default() -> Self {
        Self::empty_known()
    }
}
impl Storage {
    /// A newly constructed empty registry has two empty used extents. This is
    /// not a restore migration: serde's missing word_tail remains None.
    pub fn empty_known() -> Self {
        Self {
            slots: Vec::new(),
            last_handle: 0,
            word_tail: Some(Vec::new()),
        }
    }
    /// Import a supplied canonical lifetime mapping plus the complete used word
    /// vector. Native pointer values and epoch construction belong to the caller.
    /// Existing objects/Idle words must already match; this never registers or
    /// resets an object, creates retired lifetimes, or changes World.
    pub fn from_retained(
        world: &World,
        handles: Vec<Option<u32>>,
        mut words: Vec<u32>,
        last_handle: u32,
    ) -> Result<Self, String> {
        if handles.len() > i16::MAX as usize || words.len() < handles.len() {
            return Err("retained registry vector extents are invalid".into());
        }
        let word_tail = words.split_off(handles.len());
        let slots = handles
            .into_iter()
            .zip(words.iter().copied())
            .map(|(handle, word)| handle.map_or(Slot::Vacant(word), Slot::Occupied))
            .collect();
        let storage = Self {
            slots,
            last_handle,
            word_tail: Some(word_tail),
        };
        storage.validate(world)?;
        for (i, word) in words.into_iter().enumerate() {
            if storage.word(world, (i + 1) as ObjectId)? != word {
                return Err("retained registry live word differs from object".into());
            }
        }
        Ok(storage)
    }
    /// None is historical unknown backing, not an empty tail or pointer extent.
    pub fn word_extent(&self) -> Option<usize> {
        self.word_tail.as_ref().map(|tail| self.len() + tail.len())
    }
    /// Necessary typed-owner check for a strict captured-native ingress. The
    /// caller must separately prove source/epoch identity. General restore and
    /// the ordinary scheduler retain the existing legacy partial contract.
    pub fn require_complete_backing(&self, world: &World) -> Result<usize, String> {
        self.validate(world)?;
        self.word_extent()
            .ok_or_else(|| "complete native registry word backing is unknown".into())
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    ///44279a erases the registry pointer vector after all destructors. Lifetime
    ///handles remain process-owned and cannot be reused by subsequent ObjM IDs.
    pub fn clear_registry(&mut self) -> Result<(), String> {
        if self.slots.iter().any(|s| matches!(s, Slot::Occupied(_))) {
            return Err("registry clear still owns live objects".into());
        }
        if self.slots.is_empty() {
            return Ok(());
        }
        if let Some(tail) = self.word_tail.as_mut() {
            let mut retained = Vec::with_capacity(self.slots.len() + tail.len());
            retained.extend(self.slots.iter().map(|slot| match slot {
                Slot::Vacant(word) => *word,
                Slot::Occupied(_) => unreachable!("live slots were rejected before mutation"),
            }));
            retained.append(tail);
            *tail = retained;
        }
        // None keeps the exact old Rust clear-drop policy. It remains None:
        // clearing pointer slots cannot prove the original word tail empty.
        self.slots.clear();
        Ok(())
    }
    pub fn contains(&self, id: ObjectId) -> bool {
        self.handle(id).is_some()
    }
    pub fn handle(&self, id: ObjectId) -> Option<u32> {
        match id
            .checked_sub(1)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.slots.get(i))
        {
            Some(Slot::Occupied(handle)) => Some(*handle),
            _ => None,
        }
    }
    pub fn object_for_handle(&self, handle: u32) -> Option<ObjectId> {
        self.slots
            .iter()
            .position(|slot| *slot == Slot::Occupied(handle))
            .map(|i| (i + 1) as ObjectId)
    }
    /// A historical lifetime can be retained by a synchronous caller after
    /// unregistering it. This never resolves it to a reused script ID.
    pub fn retired_handle(&self, handle: u32) -> bool {
        handle != 0 && handle <= self.last_handle && self.object_for_handle(handle).is_none()
    }
    pub fn word(&self, world: &World, id: ObjectId) -> Result<u32, String> {
        let index = index(id)?;
        match self
            .slots
            .get(index)
            .ok_or("registry slot outside retained extent")?
        {
            Slot::Vacant(word) => Ok(*word),
            Slot::Occupied(_) => world
                .objects
                .get(&id)
                .and_then(|o| o.idle.as_ref())
                .map(IdleState::registry_word)
                .ok_or_else(|| "registered scheduler word absent".into()),
        }
    }
    /// Direct431dc9/4457ef backing access is distinct from scheduler iteration.
    /// Only an explicitly retained tail authorizes a word beyond pointer end.
    pub fn backing_word(&self, world: &World, id: ObjectId) -> Result<u32, String> {
        let index = index(id)?;
        if index < self.len() {
            return self.word(world, id);
        }
        self.word_tail
            .as_ref()
            .ok_or("registry word backing extent is unknown")?
            .get(index - self.len())
            .copied()
            .ok_or_else(|| "registry word outside retained backing".into())
    }
    fn grow(&mut self, extent: usize) -> Result<(), String> {
        if extent <= self.len() {
            return Ok(());
        }
        let delta = extent - self.len();
        if let Some(tail) = self.word_tail.as_mut() {
            // Native appends at word_end, not pointer_end. Exposed pointer
            // holes consume old tail words; appended zeros form the suffix.
            tail.resize(tail.len() + delta, 0);
            self.slots.extend(tail.drain(..delta).map(Slot::Vacant));
        } else {
            // Explicit legacy Rust policy, not a native backing proof. Keep
            // old saves' zero-gap growth without manufacturing a known tail.
            self.slots.resize(extent, Slot::Vacant(0));
        }
        Ok(())
    }
    pub fn validate(&self, world: &World) -> Result<(), String> {
        if self.len() > i16::MAX as usize {
            return Err("scheduler registry exceeds signed IDs".into());
        }
        let mut handles = std::collections::BTreeSet::new();
        for (i, slot) in self.slots.iter().enumerate() {
            let object = world.objects.get(&((i + 1) as i16));
            match (slot, object) {
                (Slot::Occupied(handle), Some(o))
                    if o.idle.is_some()
                        && *handle > 0
                        && *handle <= self.last_handle
                        && handles.insert(*handle) =>
                {
                    ()
                }
                (Slot::Vacant(_), None) => (),
                _ => return Err("scheduler registry occupancy differs from world".into()),
            }
        }
        if world
            .objects
            .keys()
            .any(|&id| id < 1 || id as usize > self.len())
        {
            return Err("world object lies outside scheduler registry".into());
        }
        Ok(())
    }
}
fn index(id: ObjectId) -> Result<usize, String> {
    if id <= 0 {
        Err("registry ID must be positive".into())
    } else {
        Ok(id as usize - 1)
    }
}
impl World {
    /// Publish native4434cd's scheduler storage immediately before inserting
    /// the object. Historical/synthetic worlds without this owner stay unknown.
    pub fn register_scheduler_object(
        &mut self,
        id: ObjectId,
        object: &mut Object,
    ) -> Result<(), String> {
        let Some(storage) = &mut self.scheduler_storage else {
            return Ok(());
        };
        let index = index(id)?;
        if self.objects.contains_key(&id) || storage.contains(id) {
            return Err("scheduler registration would replace an occupied slot".into());
        }
        let handle = storage
            .last_handle
            .checked_add(1)
            .ok_or("registry lifetime handles exhausted")?;
        storage.grow(index + 1)?;
        storage.slots[index] = Slot::Occupied(handle);
        storage.last_handle = handle;
        object.idle = Some(IdleState::from_registry_word(0xffff));
        Ok(())
    }
    /// Preserve the removed object's last word and the vector's high-water
    /// extent before the object itself is erased. Other unregister callbacks
    /// remain owned by their existing ordered removal implementation.
    pub fn unregister_scheduler_object(&mut self, id: ObjectId) -> Result<(), String> {
        let Some(storage) = &mut self.scheduler_storage else {
            return Ok(());
        };
        let index = index(id)?;
        if !storage.contains(id) {
            return Err("scheduler removal has no registered slot".into());
        }
        let word = self
            .objects
            .get(&id)
            .and_then(|o| o.idle.as_ref())
            .ok_or("removed scheduler word absent")?
            .registry_word();
        storage.slots[index] = Slot::Vacant(word);
        Ok(())
    }
    pub fn validate_scheduler_storage(&self) -> Result<(), String> {
        if let Some(storage) = &self.scheduler_storage {
            storage.validate(self)?;
        }
        Ok(())
    }
    /// Update exactly one backed word without creating an object/thread. This
    /// is not Vm::wake_object integration; the caller owns that control flow.
    pub fn set_registry_backing_word(&mut self, id: ObjectId, word: u32) -> Result<(), String> {
        self.validate_scheduler_storage()?;
        let index = index(id)?;
        let storage = self
            .scheduler_storage
            .as_mut()
            .ok_or("native registry word backing is unknown")?;
        match storage.slots.get_mut(index) {
            Some(Slot::Vacant(value)) => *value = word,
            Some(Slot::Occupied(_)) => {
                let idle = self
                    .objects
                    .get_mut(&id)
                    .and_then(|object| object.idle.as_mut())
                    .ok_or("registered scheduler word absent")?;
                // last_run is host bookkeeping and remains untouched.
                idle.countdown = word as i16;
                idle.flags = (word >> 16) as u16;
            }
            None => {
                let offset = index - storage.slots.len();
                *storage
                    .word_tail
                    .as_mut()
                    .ok_or("registry word backing extent is unknown")?
                    .get_mut(offset)
                    .ok_or("registry word outside retained backing")? = word;
            }
        }
        Ok(())
    }
    ///431dc9: replace the low half while preserving all upper flags, including
    ///a supplied backed ID with no live pointer (44574b's null-lookup tail).
    pub fn set_registry_countdown(&mut self, id: ObjectId, value: i16) -> Result<(), String> {
        let word = self
            .scheduler_storage
            .as_ref()
            .ok_or("native registry word backing is unknown")?
            .backing_word(self, id)?;
        self.set_registry_backing_word(id, (word & 0xffff0000) | u32::from(value as u16))
    }
    ///4457ef: replace only the selected upper-half bits, preserving countdown.
    pub fn set_registry_flags(
        &mut self,
        id: ObjectId,
        mask: u16,
        enabled: bool,
    ) -> Result<(), String> {
        let word = self
            .scheduler_storage
            .as_ref()
            .ok_or("native registry word backing is unknown")?
            .backing_word(self, id)?;
        let mask = u32::from(mask) << 16;
        self.set_registry_backing_word(id, (word & !mask) | if enabled { mask } else { 0 })
    }
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[rustfmt::skip]
mod word_tail_tests;
