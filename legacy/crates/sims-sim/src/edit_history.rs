//! Shared Undoable/UndoManager ownership. Commands retain their own state and
//! reference count. Vector cursors identify allocation slots, not elements:
//! callbacks can replace a slot without changing a captured cursor.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod command;
pub mod dependencies;
pub mod manager;
pub mod object;
pub mod snapshot;
pub mod tool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum List {
    Undo(u32),
    Redo(u32),
    Before(u32),
    After(u32),
    Objects(u32),
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub allocation: u32,
    pub index: usize,
}
impl Slot {
    pub fn next(self) -> Result<Self, String> {
        Ok(Self {
            index: self.index.checked_add(1).ok_or("history cursor overflow")?,
            ..self
        })
    }
    pub fn previous(self) -> Result<Self, String> {
        Ok(Self {
            index: self
                .index
                .checked_sub(1)
                .ok_or("history cursor precedes allocation")?,
            ..self
        })
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vector {
    pub allocation: u32,
    pub len: usize,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// Undoable+4,+8; both initially zero in492d63.
    pub state: u32,
    pub references: u32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub commands: BTreeMap<u32, Command>,
    /// A descriptor is the actual vector owner; allocation IDs are lifetime
    /// identities and are never reused by this Rust backend.
    #[serde(with = "list_map")]
    pub lists: BTreeMap<List, Vector>,
    /// None is uninitialized storage, Some(0) is a defined null pointer.
    /// Slots beyond a logically shortened end retain their original contents.
    pub allocations: BTreeMap<u32, Vec<Option<u32>>>,
    last_allocation: u32,
}
impl Store {
    /// Retained492d63 fields, excluding vtable/destructor services.
    pub fn construct_command(&mut self, id: u32) -> Result<(), String> {
        if id == 0
            || self.commands.contains_key(&id)
            || self.lists.contains_key(&List::Before(id))
            || self.lists.contains_key(&List::After(id))
        {
            return Err("history command identity already owned or null".into());
        }
        self.commands.insert(id, Command::default());
        self.lists.insert(List::Before(id), Vector::default());
        self.lists.insert(List::After(id), Vector::default());
        Ok(())
    }
    ///4931bf's two zero-length, zero-capacity vectors. No command is invented.
    pub fn construct_manager(&mut self, id: u32) -> Result<(), String> {
        if id == 0
            || self.lists.contains_key(&List::Undo(id))
            || self.lists.contains_key(&List::Redo(id))
        {
            return Err("history manager identity already owned or null".into());
        }
        self.lists.insert(List::Undo(id), Vector::default());
        self.lists.insert(List::Redo(id), Vector::default());
        Ok(())
    }
    pub fn command(&self, id: u32) -> Result<&Command, String> {
        self.commands
            .get(&id)
            .ok_or("history command absent".into())
    }
    pub fn command_mut(&mut self, id: u32) -> Result<&mut Command, String> {
        self.commands
            .get_mut(&id)
            .ok_or("history command absent".into())
    }
    pub fn acquire(&mut self, id: u32) -> Result<(), String> {
        let c = self.command_mut(id)?;
        c.references = c.references.wrapping_add(1);
        Ok(())
    }
    pub fn vector(&self, list: List) -> Result<&Vector, String> {
        self.lists
            .get(&list)
            .ok_or("history vector owner absent".into())
    }
    pub fn begin(&self, list: List) -> Result<Slot, String> {
        Ok(Slot {
            allocation: self.vector(list)?.allocation,
            index: 0,
        })
    }
    pub fn end(&self, list: List) -> Result<Slot, String> {
        let v = self.vector(list)?;
        Ok(Slot {
            allocation: v.allocation,
            index: v.len,
        })
    }
    pub fn read(&self, slot: Slot) -> Result<u32, String> {
        self.allocations
            .get(&slot.allocation)
            .and_then(|a| a.get(slot.index))
            .copied()
            .flatten()
            .ok_or("history cursor reads absent or undefined allocation slot".into())
    }
    pub fn write(&mut self, slot: Slot, value: u32) -> Result<(), String> {
        let target = self
            .allocations
            .get_mut(&slot.allocation)
            .and_then(|a| a.get_mut(slot.index))
            .ok_or("history cursor writes absent allocation slot")?;
        *target = Some(value);
        Ok(())
    }
    ///493069/493090's end insertion: grow by1 from empty, otherwise double.
    /// The allocator is Rust-owned; no callback occurs during these copies.
    pub fn append(&mut self, list: List, value: u32) -> Result<(), String> {
        let v = self.vector(list)?.clone();
        let capacity = if v.allocation == 0 {
            0
        } else {
            self.allocations
                .get(&v.allocation)
                .ok_or("history allocation absent")?
                .len()
        };
        if v.len > capacity {
            return Err("history end exceeds allocation".into());
        }
        let allocation = if v.len == capacity {
            let capacity = if v.len == 0 {
                1
            } else {
                v.len.checked_mul(2).ok_or("history capacity overflow")?
            };
            // Native signed pointer differences must remain representable.
            if capacity > (i32::MAX as usize / 4) {
                return Err("history allocation exceeds native address range".into());
            }
            let id = self
                .last_allocation
                .checked_add(1)
                .ok_or("history allocation identities exhausted")?;
            let mut data = Vec::new();
            data.try_reserve_exact(capacity)
                .map_err(|_| "history allocation failed")?;
            data.resize(capacity, None);
            if v.allocation != 0 {
                data[..v.len].copy_from_slice(&self.allocations[&v.allocation][..v.len]);
            }
            self.allocations.insert(id, data);
            if v.allocation != 0 {
                self.allocations.remove(&v.allocation);
            }
            self.last_allocation = id;
            id
        } else {
            v.allocation
        };
        self.write(
            Slot {
                allocation,
                index: v.len,
            },
            value,
        )?;
        *self.lists.get_mut(&list).unwrap() = Vector {
            allocation,
            len: v.len + 1,
        };
        Ok(())
    }
    ///4b5ea2 shifts the tail and shortens end; it does not release commands or
    /// zero removed pointer storage. Erase endpoints are captured by the caller.
    pub fn erase(&mut self, list: List, first: Slot, last: Slot) -> Result<(), String> {
        let v = self.vector(list)?.clone();
        if first.allocation != v.allocation
            || last.allocation != v.allocation
            || first.index > last.index
            || last.index > v.len
        {
            return Err("history erase endpoints lost their allocation".into());
        }
        if first.index == last.index {
            return Ok(());
        }
        let a = self
            .allocations
            .get_mut(&v.allocation)
            .ok_or("history erase allocation absent")?;
        if v.len > a.len() {
            return Err("history erase end exceeds allocation".into());
        }
        a.copy_within(last.index..v.len, first.index);
        self.lists.get_mut(&list).unwrap().len -= last.index - first.index;
        Ok(())
    }
    pub fn pop_end(&mut self, list: List) -> Result<(), String> {
        let v = self.lists.get_mut(&list).ok_or("history vector absent")?;
        v.len = v
            .len
            .checked_sub(1)
            .ok_or("history end decrement on empty vector")?;
        Ok(())
    }
    ///527c7a destroys the vector allocation; it does not release its elements.
    ///The owner subobject ceases to exist, so captured cursors cannot silently
    ///read newly allocated storage at the same address after destruction.
    pub fn destroy_vector(&mut self, list: List) -> Result<(), String> {
        let v = self
            .lists
            .remove(&list)
            .ok_or("history vector owner absent")?;
        self.allocations.remove(&v.allocation);
        Ok(())
    }
    /// Allocator tail after the concrete destructor has emptied its vectors.
    /// This does not substitute for virtual destruction or release children.
    pub fn deallocate_command(&mut self, id: u32) -> Result<(), String> {
        self.command(id)?;
        let keys = [List::Before(id), List::After(id), List::Objects(id)];
        if keys
            .iter()
            .filter_map(|key| self.lists.get(key))
            .any(|v| v.len != 0)
        {
            return Err("command destructor left owned children".into());
        }
        for key in keys {
            if let Some(v) = self.lists.remove(&key) {
                self.allocations.remove(&v.allocation);
            }
        }
        self.commands.remove(&id);
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut owned = std::collections::BTreeSet::new();
        for v in self.lists.values() {
            if v.allocation == 0 {
                if v.len != 0 {
                    return Err("null history vector has nonzero end".into());
                }
            } else {
                let a = self
                    .allocations
                    .get(&v.allocation)
                    .ok_or("history allocation absent")?;
                if !owned.insert(v.allocation)
                    || v.allocation > self.last_allocation
                    || v.len > a.len()
                    || a[..v.len].iter().any(Option::is_none)
                {
                    return Err("invalid history vector ownership or initialized extent".into());
                }
            }
        }
        if owned.len() != self.allocations.len() || self.commands.contains_key(&0) {
            return Err("unowned history allocation or null command identity".into());
        }
        Ok(())
    }
}

mod list_map {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        map: &BTreeMap<List, Vector>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<List, Vector>, D::Error> {
        let entries = Vec::<(List, Vector)>::deserialize(deserializer)?;
        let mut map = BTreeMap::new();
        for (key, value) in entries {
            if map.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate history vector owner"));
            }
        }
        Ok(map)
    }
}
