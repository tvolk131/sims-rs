//! Per-call borrowed physical-link transaction. Planning reads the latest staged
//! values in native operation order; successful plans commit that same write
//! order. Registry membership and allocation extents never change in this layer.
use super::{Descriptor, Links};
use sims_vm::{ObjectId, World, autonomy::Placement};

#[derive(Clone, Copy)]
enum Edit {
    Head(usize, usize, ObjectId),
    Next(ObjectId, ObjectId),
    Slot(ObjectId, usize, Option<ObjectId>),
    Word(ObjectId, i16, i16),
    Level(ObjectId, i32),
}
// Apply has no loop: at most three unlinks (three writes each), one raw
// attachment (four writes), or an insertion with two attachments (eleven).
// Keep head/word writes separate even when aliases overwrite the same field.
const MAX_EDITS: usize = 16;
struct Plan<'a> {
    links: &'a Links,
    world: &'a World,
    edits: [Option<Edit>; MAX_EDITS],
    len: usize,
}
impl<'a> Plan<'a> {
    fn new(links: &'a Links, world: &'a World) -> Self {
        Self {
            links,
            world,
            edits: [None; MAX_EDITS],
            len: 0,
        }
    }
    fn push(&mut self, edit: Edit) {
        self.edits[self.len] = Some(edit);
        self.len += 1;
    }
    fn latest<T>(&self, mut find: impl FnMut(Edit) -> Option<T>) -> Option<T> {
        self.edits[..self.len]
            .iter()
            .rev()
            .flatten()
            .find_map(|&edit| find(edit))
    }
    fn position(&self, id: ObjectId) -> Result<Placement, String> {
        let mut p = Links::position(self.world, id)?;
        if let Some(level) = self.latest(|e| match e {
            Edit::Level(i, v) if i == id => Some(v),
            _ => None,
        }) {
            p.level = level;
        }
        Ok(p)
    }
    fn word(&self, id: ObjectId, field: i16) -> Result<i16, String> {
        self.latest(|e| match e {
            Edit::Word(i, f, v) if (i, f) == (id, field) => Some(v),
            _ => None,
        })
        .map(Ok)
        .unwrap_or_else(|| Links::word(self.world, id, field))
    }
    fn slot(&self, parent: ObjectId, slot: i32) -> Result<Option<ObjectId>, String> {
        let id = if slot == 0 {
            self.latest(|e| match e {
                Edit::Next(i, v) if i == parent => Some(v),
                _ => None,
            })
            .map(Ok)
            .unwrap_or_else(|| {
                self.links
                    .next
                    .get(&parent)
                    .copied()
                    .ok_or("native sentinel link absent")
            })?
        } else {
            let entries = &self
                .world
                .objects
                .get(&parent)
                .ok_or("native physical slot outside allocation")?
                .slots;
            let index = usize::try_from(slot - 1)
                .ok()
                .filter(|&i| i < entries.len())
                .ok_or("native physical slot outside allocation")?;
            self.latest(|e| match e {
                Edit::Slot(i, s, v) if (i, s) == (parent, index) => Some(v),
                _ => None,
            })
            .unwrap_or(entries[index])
            .unwrap_or(0)
        };
        Ok(Links::registered(self.world, id))
    }
    fn write_slot(&mut self, parent: ObjectId, slot: i32, id: ObjectId) -> Result<(), String> {
        if slot == 0 {
            if !self.links.next.contains_key(&parent) {
                return Err("native sentinel link absent".into());
            }
            self.push(Edit::Next(parent, id));
        } else {
            let entries = &self
                .world
                .objects
                .get(&parent)
                .ok_or("native physical slot outside allocation")?
                .slots;
            let index = usize::try_from(slot - 1)
                .ok()
                .filter(|&i| i < entries.len())
                .ok_or("native physical slot outside allocation")?;
            self.push(Edit::Slot(parent, index, (id != 0).then_some(id)));
        }
        Ok(())
    }
    fn words(&mut self, id: ObjectId, slot: i16, predecessor: ObjectId) -> Result<(), String> {
        if !self.world.objects.contains_key(&id) {
            return Err("link object absent".into());
        }
        self.push(Edit::Word(id, 14, slot));
        self.push(Edit::Word(id, 26, predecessor));
        Ok(())
    }
    fn capture_raw(&self, id: ObjectId) -> Result<Descriptor, String> {
        let p = self.position(id)?;
        Ok(Descriptor {
            on_lot: self.links.on_lot([p.x, p.y]),
            position: [p.x, p.y],
            container: Links::registered(self.world, self.word(id, 26)?),
            slot: i32::from(self.word(id, 14)?),
            level: p.level,
        })
    }
    fn unlink(&mut self, id: ObjectId) -> Result<(), String> {
        let old = self.capture_raw(id)?;
        if let Some(parent) = old.container {
            self.write_slot(parent, old.slot, 0)?;
        } else if old.on_lot {
            if let Some((floor, cell)) = self.links.index(old.position, old.level) {
                self.push(Edit::Head(floor, cell, 0));
            }
        }
        // capture_raw already established object presence via both word reads.
        self.words(id, -1, 0)?;
        Ok(())
    }
    fn attach_raw(&mut self, id: ObjectId, d: Descriptor) -> Result<(), String> {
        if !d.on_lot {
            self.unlink(id)?;
        }
        if let Some(parent) = d.container {
            self.write_slot(parent, d.slot, id)?;
            self.words(id, d.slot as i16, parent)?;
        } else {
            if !d.on_lot {
                return Ok(());
            }
            if let Some((floor, cell)) = self.links.index(d.position, d.level) {
                self.push(Edit::Head(floor, cell, id));
            }
            self.words(id, -1, 0)?;
        }
        self.position(id).map_err(|_| "link position absent")?;
        self.push(Edit::Level(id, d.level));
        Ok(())
    }
    fn at(&self, d: Descriptor) -> Result<Option<ObjectId>, String> {
        if !d.on_lot {
            return Ok(None);
        }
        if let Some(parent) = d.container {
            return self.slot(parent, d.slot);
        }
        Ok(self
            .links
            .index(d.position, d.level)
            .and_then(|(floor, cell)| {
                let id = self
                    .latest(|e| match e {
                        Edit::Head(f, c, v) if (f, c) == (floor, cell) => Some(v),
                        _ => None,
                    })
                    .unwrap_or(self.links.heads[floor][cell]);
                Links::registered(self.world, id)
            }))
    }
    fn apply(&mut self, id: ObjectId, d: Descriptor) -> Result<(), String> {
        if !d.on_lot {
            let old = self.capture_raw(id)?;
            self.unlink(id)?;
            if let Some(next) = self.slot(id, 0)? {
                self.unlink(next)?;
                self.attach_raw(next, old)?;
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
            let existing = self.at(raw)?;
            if existing != Some(id) {
                if let Some(existing) = existing {
                    self.unlink(existing)?;
                }
                self.attach_raw(id, raw)?;
                if let Some(existing) = existing {
                    self.attach_raw(
                        existing,
                        Descriptor {
                            on_lot: true,
                            position: raw.position,
                            container: Some(id),
                            slot: 0,
                            level: self.position(id)?.level,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn apply(
    links: &mut Links,
    world: &mut World,
    id: ObjectId,
    d: Descriptor,
) -> Result<(), String> {
    let edits = {
        let mut plan = Plan::new(links, world);
        plan.apply(id, d)?;
        plan.edits
    };
    // Each destination was checked during planning. Exclusive borrows ensure
    // membership/extents cannot change before commit; commit has no Result path.
    for edit in edits.into_iter().flatten() {
        match edit {
            Edit::Head(f, c, id) => links.heads[f][c] = id,
            Edit::Next(parent, id) => *links.next.get_mut(&parent).unwrap() = id,
            Edit::Slot(parent, index, id) => {
                world.objects.get_mut(&parent).unwrap().slots[index] = id
            }
            Edit::Word(id, field, value) => {
                world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .data
                    .insert(field, value);
            }
            Edit::Level(id, level) => world.positions.get_mut(&id).unwrap().level = level,
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn preview(
    links: &Links,
    world: &World,
    id: ObjectId,
    d: Descriptor,
) -> (Result<(), String>, usize) {
    let mut plan = Plan::new(links, world);
    let result = plan.apply(id, d);
    (result, plan.len)
}
