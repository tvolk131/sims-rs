//! Native dynamic-group joining (4235ee) and function-8 updates (4234ab/d8).
//! Physical iteration includes contained objects. Group membership and the next
//! member are read again after each callback, never frozen into an object list.
use crate::lot_links::Links;
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, World};
use std::collections::BTreeSet;

pub mod live;

fn neighbors(
    links: &Links,
    world: &World,
    position: [i32; 2],
    level: i32,
    direction: usize,
) -> Result<Vec<ObjectId>, String> {
    let (dx, dy) = crate::routing::DIRECTION_OFFSETS[direction & 7];
    let tile = [
        (position[1].wrapping_add(dy) >> 4) as i8,
        (position[0].wrapping_add(dx) >> 4) as i8,
        level as i8,
    ];
    links
        .head(world, tile)
        .map_or_else(|| Ok(vec![]), |id| links.walk(world, id))
}

/// Joining changes only multipart links/offsets/facing, so physical traversal
/// remains stable throughout each callback-free neighboring-tile scan.
pub fn join_neighbors(links: &Links, world: &mut World, selected: ObjectId) -> Result<(), String> {
    let q = *world
        .positions
        .get(&selected)
        .ok_or("dynamic placement position absent")?;
    let definition = world
        .objects
        .get(&selected)
        .and_then(|o| o.definition.as_ref())
        .ok_or("dynamic placement resource absent")?
        .clone();
    for direction in [0, 2, 4, 6] {
        for neighbor in neighbors(links, world, [q.x, q.y], q.level, direction)? {
            let object = &world.objects[&neighbor];
            if object
                .definition
                .as_ref()
                .is_some_and(|d| d.source == definition.source && d.object == definition.object)
            {
                if object.multitile.is_none() {
                    return Err("dynamic neighbor resource has non-multipart runtime type".into());
                }
                // Native this is the NEIGHBOR: the newly placed group is
                // appended to that neighbor's current group, not vice versa.
                sims_vm::multitile::merge(world, neighbor, selected)?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Update {
    pub object: ObjectId,
    pub args: [i16; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Updates {
    selected: ObjectId,
    current: Option<ObjectId>,
    advance: bool,
    seen: BTreeSet<ObjectId>,
}
impl Updates {
    pub fn begin(links: &Links, world: &mut World, selected: ObjectId) -> Result<Self, String> {
        let current = if links.capture(world, selected)?.on_lot {
            Some(
                world.objects[&selected]
                    .multitile
                    .as_ref()
                    .ok_or("dynamic update part absent")?
                    .head
                    .unwrap_or(selected),
            )
        } else {
            None
        };
        Ok(Self {
            selected,
            current,
            advance: false,
            seen: BTreeSet::new(),
        })
    }
    pub fn binding(&self) -> ObjectId {
        self.selected
    }
    pub fn pending(&self) -> Option<ObjectId> {
        self.advance.then_some(self.current).flatten()
    }
    pub fn complete(&self) -> bool {
        self.current.is_none()
    }
    pub fn validate(&self, world: &World) -> Result<(), String> {
        if !world.objects.contains_key(&self.selected)
            || self.current.is_some_and(|id| {
                !world.objects.contains_key(&id) || self.advance != self.seen.contains(&id)
            })
            || self.current.is_none() && self.advance
        {
            return Err("invalid dynamic update cursor".into());
        }
        Ok(())
    }
    pub fn next(&mut self, links: &Links, world: &mut World) -> Result<Option<Update>, String> {
        loop {
            if self.advance {
                self.current = world
                    .objects
                    .get(&self.current.unwrap())
                    .and_then(|o| o.multitile.as_ref())
                    .ok_or("dynamic callback removed active part")?
                    .next;
                self.advance = false;
            }
            let Some(id) = self.current else {
                return Ok(None);
            };
            if !self.seen.insert(id) {
                return Err("cyclic dynamic update traversal".into());
            }
            self.advance = true;
            if !links.capture(world, id)?.on_lot {
                continue;
            }
            let q = world.positions[&id];
            let direction = *world.objects[&id]
                .data
                .get(&1)
                .ok_or("dynamic update direction absent")?;
            let members = sims_vm::multitile::chain(world, id)?;
            let mut mask = 0;
            for bit in 0..8 {
                let at = (i32::from(direction) + bit) as usize & 7;
                if neighbors(links, world, [q.x, q.y], q.level, at)?
                    .iter()
                    .any(|n| members.contains(n))
                {
                    mask |= 1 << bit;
                }
            }
            return Ok(Some(Update {
                object: id,
                args: [mask, 0, 0, 0],
            }));
        }
    }
}

/// Native 42346a remembers the surviving head before unlinking. It updates
/// that group first, then rechecks the selected object's live on-lot state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Split {
    selected: ObjectId,
    other: Option<ObjectId>,
    phase: u8,
    updates: Option<Updates>,
}
impl Split {
    pub fn begin(world: &mut World, selected: ObjectId) -> Result<Self, String> {
        let chain = sims_vm::multitile::chain(world, selected)?;
        let other = if chain[0] == selected {
            chain.get(1).copied()
        } else {
            Some(chain[0])
        };
        sims_vm::multitile::unlink(world, selected)?;
        Ok(Self {
            selected,
            other,
            phase: 0,
            updates: None,
        })
    }
    pub fn binding(&self) -> ObjectId {
        self.selected
    }
    pub fn pending(&self) -> Option<ObjectId> {
        self.updates.as_ref().and_then(Updates::pending)
    }
    pub fn complete(&self) -> bool {
        self.phase == 3
    }
    pub fn validate(&self, world: &World) -> Result<(), String> {
        if self.phase > 3
            || !world.objects.contains_key(&self.selected)
            || self.other == Some(self.selected)
            || self.phase == 2 && self.updates.is_none()
            || self.phase == 0
                && self
                    .other
                    .is_some_and(|id| !world.objects.contains_key(&id))
            || self.updates.is_some() && !matches!(self.phase, 0 | 2)
        {
            return Err("invalid dynamic split continuation".into());
        }
        if let Some(updates) = &self.updates {
            updates.validate(world)?;
            if Some(updates.binding())
                != if self.phase == 0 {
                    self.other
                } else {
                    Some(self.selected)
                }
            {
                return Err("dynamic split update binding differs".into());
            }
        }
        Ok(())
    }
    pub fn next(&mut self, links: &Links, world: &mut World) -> Result<Option<Update>, String> {
        loop {
            if let Some(updates) = &mut self.updates {
                if let Some(update) = updates.next(links, world)? {
                    return Ok(Some(update));
                }
                self.updates = None;
                self.phase += 1;
            }
            match self.phase {
                0 => {
                    if let Some(other) = self.other {
                        self.updates = Some(Updates::begin(links, world, other)?);
                    } else {
                        self.phase = 1;
                    }
                }
                1 => {
                    // This explicit 432595 precedes 4234ab's own capture.
                    if links.capture(world, self.selected)?.on_lot {
                        self.updates = Some(Updates::begin(links, world, self.selected)?);
                        self.phase = 2;
                    } else {
                        self.phase = 3;
                    }
                }
                3 => return Ok(None),
                _ => return Err("dynamic split lost its update traversal".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests;
