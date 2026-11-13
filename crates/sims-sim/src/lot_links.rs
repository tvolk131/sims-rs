//! Native physical placement links used while detaching/restoring loaded objects.
//! Physical slot zero links objects sharing a tile or containment slot; script
//! slots start at physical slot one. This layer does not run callbacks, validate
//! placement permission, move coordinates, or publish logical container fields.
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, World, autonomy::Placement};
use std::collections::{BTreeMap, BTreeSet};

/// Per-traversal scratch, never saved or shared across simulation slices. Most
/// physical chains/ancestor climbs contain only a few IDs. Larger traversals
/// use the complete i16 domain, including invalid IDs checked by the caller.
#[derive(Default)]
struct Seen {
    small: [ObjectId; 16],
    len: usize,
    bits: Option<Box<[u64; 1024]>>,
}
impl Seen {
    fn insert(&mut self, id: ObjectId) -> bool {
        if self.bits.is_none() {
            if self.small[..self.len].contains(&id) {
                return false;
            }
            if self.len < self.small.len() {
                self.small[self.len] = id;
                self.len += 1;
                return true;
            }
            let mut bits = Box::new([0; 1024]);
            for old in self.small {
                let i = usize::from(old as u16);
                bits[i / 64] |= 1 << (i % 64);
            }
            self.bits = Some(bits);
        }
        let i = usize::from(id as u16);
        let word = &mut self.bits.as_mut().unwrap()[i / 64];
        let mask = 1 << (i % 64);
        let fresh = *word & mask == 0;
        *word |= mask;
        fresh
    }
    fn clear(&mut self) {
        self.len = 0;
        if let Some(bits) = &mut self.bits {
            bits.fill(0);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Links {
    pub size: usize,
    pub heads: Vec<Vec<i16>>,
    pub next: BTreeMap<ObjectId, i16>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Descriptor {
    pub on_lot: bool,
    pub position: [i32; 2],
    pub container: Option<ObjectId>,
    /// Logical (zero based script) slot except in explicitly raw operations.
    pub slot: i32,
    pub level: i32,
}
/// Native 0x43f364/0x43f379 iterator. Advance after servicing the current
/// object's callback: its live slots and predecessor may have changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cursor {
    root: ObjectId,
    pub current: Option<ObjectId>,
}
impl Cursor {
    pub fn new(root: ObjectId) -> Self {
        Self {
            root,
            current: Some(root),
        }
    }
    pub fn advance(&mut self, links: &Links, world: &World) -> Result<(), String> {
        let Some(mut node) = self.current else {
            return Ok(());
        };
        let mut slot = 0;
        let mut seen = Seen::default();
        loop {
            if !seen.insert(node) {
                return Err("cyclic physical iterator ancestry".into());
            }
            // No callback or world mutation occurs within one advance. Borrow
            // this parent once, while still checking its sentinel before slots.
            let object = world
                .objects
                .get(&node)
                .ok_or("physical iterator object absent")?;
            let count = object.slots.len() + 1;
            while slot < count as i32 {
                let id = if slot == 0 {
                    *links.next.get(&node).ok_or("native sentinel link absent")?
                } else {
                    usize::try_from(slot - 1)
                        .ok()
                        .and_then(|index| object.slots.get(index))
                        .ok_or("native physical slot outside allocation")?
                        .unwrap_or(0)
                };
                let child = Links::registered(world, id);
                slot += 1;
                if child.is_some() {
                    self.current = child;
                    return Ok(());
                }
            }
            self.current = None;
            if node == self.root {
                return Ok(());
            }
            let predecessor = *object
                .data
                .get(&26)
                .ok_or("native physical link word absent")?;
            let parent = Links::registered(world, predecessor)
                .ok_or("physical iterator lost its predecessor")?;
            slot = i32::from(
                *object
                    .data
                    .get(&14)
                    .ok_or("native physical link word absent")?,
            ) + 1;
            if slot < 0 {
                return Err("physical iterator has a negative slot".into());
            }
            node = parent;
        }
    }
}
impl Links {
    /// 0x43244d: first blocking object in the tile's physical slot-zero chain.
    /// Logical children are not traversed and a non-seat blocker ends lookup.
    pub fn first_blocker(
        &self,
        world: &World,
        point: [i32; 2],
        level: i32,
    ) -> Result<Option<ObjectId>, String> {
        let mut current = self.head(
            world,
            [(point[1] >> 4) as i8, (point[0] >> 4) as i8, level as i8],
        );
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err("cyclic seat lookup chain".into());
            }
            let flags = Self::word(world, id, 8)? as u16;
            if if flags & 4 == 0 {
                flags & 16 == 0
            } else {
                flags & 2 != 0
            } {
                return Ok(Some(id));
            }
            current = self.slot(world, id, 0)?;
        }
        Ok(None)
    }
    pub fn head(&self, world: &World, tile: [i8; 3]) -> Option<ObjectId> {
        let [column, row, level] = tile;
        self.index(
            [i32::from(row) * 16, i32::from(column) * 16],
            i32::from(level),
        )
        .and_then(|(floor, cell)| Self::registered(world, self.heads[floor][cell]))
    }
    pub fn walk(&self, world: &World, root: ObjectId) -> Result<Vec<ObjectId>, String> {
        let mut result = vec![];
        self.walk_into(world, root, &mut result, &mut Seen::default())?;
        Ok(result)
    }
    fn walk_into(
        &self,
        world: &World,
        root: ObjectId,
        result: &mut Vec<ObjectId>,
        seen: &mut Seen,
    ) -> Result<(), String> {
        result.clear();
        seen.clear();
        let mut cursor = Cursor::new(root);
        while let Some(id) = cursor.current {
            if !seen.insert(id) {
                return Err("cyclic or duplicate physical slot traversal".into());
            }
            result.push(id);
            cursor.advance(self, world)?;
        }
        Ok(())
    }
    /// Reconstruct native head/physical-slot order using borrowed world state.
    /// Keep all traversal checks before comparing or publishing the result, so
    /// malformed links retain the same diagnostic ordering as publication.
    pub(crate) fn collect_tile_chains(&self, world: &World) -> Result<Vec<Vec<ObjectId>>, String> {
        let mut chains = vec![];
        self.for_each_tile_chain(world, |chain| chains.push(chain.to_vec()))?;
        Ok(chains)
    }
    fn for_each_tile_chain(
        &self,
        world: &World,
        mut visit: impl FnMut(&[ObjectId]),
    ) -> Result<(), String> {
        self.validate_arrays()?;
        let active = world
            .map_size
            .map(|map| map.checked_size(self.size))
            .transpose()?
            .unwrap_or(self.size);
        let mut seen = Seen::default();
        let mut chain_seen = Seen::default();
        let mut chain = vec![];
        //Native backing heads outside world+10 may be stale saved references.
        //Keep them physically, without publishing active script tile entries.
        for head in self.heads.iter().flat_map(|floor| {
            floor
                .chunks_exact(self.size)
                .take(active)
                .flat_map(move |column| column[..active].iter())
        }) {
            if let Some(id) = Self::registered(world, *head) {
                self.walk_into(world, id, &mut chain, &mut chain_seen)?;
                if chain.iter().any(|&id| !seen.insert(id)) {
                    return Err("object occurs in multiple native tile chains".into());
                }
                visit(&chain);
            }
        }
        Ok(())
    }
    /// Compare without constructing an owned copy of every chain. A mismatch
    /// is reported only after the complete traversal: malformed links and a
    /// missing person registry must retain their original diagnostic priority.
    pub fn validate_tile_chains(&self, world: &World) -> Result<(), String> {
        let expected = world.person.as_ref().map(|p| &p.tile_chains);
        let mut count = 0;
        let mut equal = true;
        self.for_each_tile_chain(world, |chain| {
            equal &= expected
                .and_then(|chains| chains.get(count))
                .map(Vec::as_slice)
                == Some(chain);
            count += 1;
        })?;
        let expected = expected.ok_or("tile registry unavailable")?;
        if !equal || count != expected.len() {
            return Err("native runtime physical tile order mismatch".into());
        }
        Ok(())
    }
    /// Publish the native head/physical-slot order for existing VM queries.
    /// This never infers ordering from positions or numeric object IDs.
    pub fn publish_tile_chains(&self, world: &mut World) -> Result<(), String> {
        let chains = self.collect_tile_chains(world)?;
        world
            .person
            .as_mut()
            .ok_or("tile registry unavailable")?
            .tile_chains = chains;
        Ok(())
    }
    pub fn validate_arrays(&self) -> Result<(), String> {
        if !(2..=64).contains(&self.size)
            || !(1..=2).contains(&self.heads.len())
            || self.heads.iter().any(|h| h.len() != self.size * self.size)
        {
            return Err("invalid native placement link arrays".into());
        }
        Ok(())
    }
    fn index(&self, position: [i32; 2], level: i32) -> Option<(usize, usize)> {
        let [column, row] = [(position[1] >> 4) as i8, (position[0] >> 4) as i8];
        (column >= 0
            && row >= 0
            && (column as usize) < self.size
            && (row as usize) < self.size
            && level >= 1
            && (level as usize) <= self.heads.len())
        .then(|| {
            (
                level as usize - 1,
                column as usize * self.size + row as usize,
            )
        })
    }
    pub(crate) fn on_lot(&self, position: [i32; 2]) -> bool {
        let [column, row] = [(position[1] >> 4) as i8, (position[0] >> 4) as i8];
        column > 0 && row > 0 && (column as usize) < self.size - 1 && (row as usize) < self.size - 1
    }
    fn position(world: &World, id: ObjectId) -> Result<Placement, String> {
        world
            .positions
            .get(&id)
            .copied()
            .ok_or("native link object position absent".into())
    }
    fn word(world: &World, id: ObjectId, index: i16) -> Result<i16, String> {
        world
            .objects
            .get(&id)
            .and_then(|o| o.data.get(&index))
            .copied()
            .ok_or_else(|| "native physical link word absent".into())
    }
    fn registered(world: &World, id: i16) -> Option<ObjectId> {
        (id > 0 && world.objects.contains_key(&id)).then_some(id)
    }
    pub(crate) fn slot(
        &self,
        world: &World,
        parent: ObjectId,
        slot: i32,
    ) -> Result<Option<ObjectId>, String> {
        let id = if slot == 0 {
            *self
                .next
                .get(&parent)
                .ok_or("native sentinel link absent")?
        } else {
            world
                .objects
                .get(&parent)
                .and_then(|o| usize::try_from(slot - 1).ok().and_then(|i| o.slots.get(i)))
                .ok_or("native physical slot outside allocation")?
                .unwrap_or(0)
        };
        Ok(Self::registered(world, id))
    }
    fn write_slot(
        &mut self,
        world: &mut World,
        parent: ObjectId,
        slot: i32,
        id: ObjectId,
    ) -> Result<(), String> {
        if slot == 0 {
            *self
                .next
                .get_mut(&parent)
                .ok_or("native sentinel link absent")? = id;
        } else {
            *world
                .objects
                .get_mut(&parent)
                .and_then(|o| {
                    usize::try_from(slot - 1)
                        .ok()
                        .and_then(|i| o.slots.get_mut(i))
                })
                .ok_or("native physical slot outside allocation")? = (id != 0).then_some(id);
        }
        Ok(())
    }
    /// 0x432844: raw predecessor/physical-slot descriptor.
    pub fn capture_raw(&self, world: &World, id: ObjectId) -> Result<Descriptor, String> {
        let p = Self::position(world, id)?;
        Ok(Descriptor {
            on_lot: self.on_lot([p.x, p.y]),
            position: [p.x, p.y],
            container: Self::registered(world, Self::word(world, id, 26)?),
            slot: i32::from(Self::word(world, id, 14)?),
            level: p.level,
        })
    }
    /// 0x4325b5 follows zero-slot predecessors to find the logical container.
    /// An unresolved zero-slot predecessor normalizes that node's slot to -1.
    pub fn capture(&self, world: &mut World, id: ObjectId) -> Result<Descriptor, String> {
        let p = Self::position(world, id)?;
        let on_lot = self.on_lot([p.x, p.y]);
        let mut root = id;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(root) {
                return Err("cyclic zero-slot predecessor chain".into());
            }
            if Self::word(world, root, 14)? != 0 {
                break;
            }
            let Some(parent) = Self::registered(world, Self::word(world, root, 26)?) else {
                break;
            };
            root = parent;
        }
        let physical = Self::word(world, root, 14)?;
        let slot = if physical == -1 {
            -1
        } else {
            i32::from(physical) - 1
        };
        let container = Self::registered(world, Self::word(world, root, 26)?);
        if physical == 0 {
            world.objects.get_mut(&root).unwrap().data.insert(14, -1);
        }
        Ok(Descriptor {
            on_lot,
            position: if on_lot { [p.x, p.y] } else { [-16, -16] },
            container,
            slot,
            level: p.level,
        })
    }
    pub(crate) fn unlink(&mut self, world: &mut World, id: ObjectId) -> Result<(), String> {
        let old = self.capture_raw(world, id)?;
        if let Some(parent) = old.container {
            self.write_slot(world, parent, old.slot, 0)?;
        } else if old.on_lot {
            if let Some((floor, cell)) = self.index(old.position, old.level) {
                self.heads[floor][cell] = 0;
            }
        }
        world
            .objects
            .get_mut(&id)
            .unwrap()
            .data
            .extend([(14, -1), (26, 0)]);
        Ok(())
    }
    #[cfg(test)]
    fn attach_raw(&mut self, world: &mut World, id: ObjectId, d: Descriptor) -> Result<(), String> {
        if !d.on_lot {
            self.unlink(world, id)?;
        }
        if let Some(parent) = d.container {
            self.write_slot(world, parent, d.slot, id)?;
            world
                .objects
                .get_mut(&id)
                .ok_or("link object absent")?
                .data
                .extend([(14, d.slot as i16), (26, parent)]);
        } else {
            if !d.on_lot {
                return Ok(());
            }
            if let Some((floor, cell)) = self.index(d.position, d.level) {
                self.heads[floor][cell] = id;
            }
            world
                .objects
                .get_mut(&id)
                .ok_or("link object absent")?
                .data
                .extend([(14, -1), (26, 0)]);
        }
        world
            .positions
            .get_mut(&id)
            .ok_or("link position absent")?
            .level = d.level;
        Ok(())
    }
    #[cfg(test)]
    fn at(&self, world: &World, d: Descriptor) -> Result<Option<ObjectId>, String> {
        if !d.on_lot {
            return Ok(None);
        }
        if let Some(parent) = d.container {
            return self.slot(world, parent, d.slot);
        }
        Ok(self
            .index(d.position, d.level)
            .and_then(|(floor, cell)| Self::registered(world, self.heads[floor][cell])))
    }
    /// Apply branch of 0x4329c1. Location acceptance is a separate operation.
    /// Native callers unlink before insertion; insertion itself does not remove
    /// the object from another head. No coordinates or script fields 2/3 change.
    pub fn apply(
        &mut self,
        world: &mut World,
        id: ObjectId,
        descriptor: Descriptor,
    ) -> Result<(), String> {
        self.validate_arrays()?;
        transaction::apply(self, world, id, descriptor)
    }
    #[cfg(test)]
    fn apply_inner(
        &mut self,
        world: &mut World,
        id: ObjectId,
        d: Descriptor,
    ) -> Result<(), String> {
        if !d.on_lot {
            let old = self.capture_raw(world, id)?;
            self.unlink(world, id)?;
            if let Some(next) = self.slot(world, id, 0)? {
                self.unlink(world, next)?;
                self.attach_raw(world, next, old)?;
            }
        } else {
            let raw = Descriptor {
                slot: if d.slot == -1 {
                    -1
                } else {
                    d.slot.checked_add(1).ok_or("physical slot overflow")?
                },
                ..d
            };
            let existing = self.at(world, raw)?;
            if existing != Some(id) {
                if let Some(existing) = existing {
                    self.unlink(world, existing)?;
                }
                self.attach_raw(world, id, raw)?;
                if let Some(existing) = existing {
                    self.attach_raw(
                        world,
                        existing,
                        Descriptor {
                            on_lot: true,
                            position: raw.position,
                            container: Some(id),
                            slot: 0,
                            level: Self::position(world, id)?.level,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "lot_links/traversal_tests.rs"]
mod traversal_tests;

#[path = "lot_links/transaction.rs"]
mod transaction;

#[cfg(test)]
#[path = "lot_links/transaction_tests.rs"]
mod transaction_tests;

#[cfg(test)]
#[path = "lot_links/cursor_lookup_tests.rs"]
mod cursor_lookup_tests;
