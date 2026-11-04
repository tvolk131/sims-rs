//! Native 0x433612 seat-facing updates against physical placement links.
//! The host runs requested callback 9 before asking for the next update; that
//! callback may alter the live slots that the native iterator visits next.
use crate::lot_links::{Cursor, Links};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, World};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Update {
    pub object: ObjectId,
    pub flags: u16,
    pub callback: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Adjacency {
    cursor: Cursor,
    seat: bool,
    advance: bool,
    seen: BTreeSet<ObjectId>,
}
fn field(world: &World, id: ObjectId, index: i16) -> Result<i16, String> {
    world
        .objects
        .get(&id)
        .and_then(|o| o.data.get(&index))
        .copied()
        .ok_or("adjacency data missing".into())
}
fn function(world: &World, p: &Program, id: ObjectId, index: usize) -> Result<bool, String> {
    let d = world
        .objects
        .get(&id)
        .and_then(|o| o.definition.as_ref())
        .ok_or("adjacency definition missing")?;
    let table = p
        .modules
        .get(&d.source)
        .and_then(|m| m.object_functions.get(&d.object))
        .ok_or("adjacency function table missing")?;
    Ok(table.get(index).is_some_and(|f| f.action != 0))
}
impl Adjacency {
    pub(crate) fn callback_object(&self) -> Option<ObjectId> {
        self.advance.then_some(self.cursor.current).flatten()
    }
    pub fn begin(
        links: &Links,
        world: &World,
        p: &Program,
        tile: [i8; 3],
    ) -> Result<Option<Self>, String> {
        links.validate_arrays()?;
        let inside = |[x, y, _]: [i8; 3]| {
            x > 0 && y > 0 && (x as usize) < links.size - 1 && (y as usize) < links.size - 1
        };
        if !inside(tile) {
            return Ok(None);
        }
        let Some(root) = links.head(world, tile) else {
            return Ok(None);
        };
        let mut seat = false;
        for d in [0usize, 2, 4, 6] {
            let (dx, dy) = crate::routing::DIRECTION_OFFSETS[d];
            let other = [
                tile[0].wrapping_add((dy / 16) as i8),
                tile[1].wrapping_add((dx / 16) as i8),
                tile[2],
            ];
            if !inside(other) {
                continue;
            }
            let mut current = links.head(world, other);
            let mut visited = BTreeSet::new();
            while let Some(id) = current {
                if !visited.insert(id) {
                    return Err("cyclic adjacent tile chain".into());
                }
                if field(world, id, 8)? & 4 == 0 {
                    break;
                }
                current = links.slot(world, id, 0)?;
            }
            if let Some(id) = current {
                if function(world, p, id, 26)?
                    && field(world, id, 1)? as u16 == ((d as u16 + 4) & 7)
                {
                    seat = true;
                    break;
                }
            }
        }
        Ok(Some(Self {
            cursor: Cursor::new(root),
            seat,
            advance: false,
            seen: BTreeSet::new(),
        }))
    }
    pub fn next(
        &mut self,
        links: &Links,
        world: &mut World,
        p: &Program,
    ) -> Result<Option<Update>, String> {
        if self.advance {
            self.cursor.advance(links, world)?;
        }
        let Some(id) = self.cursor.current else {
            return Ok(None);
        };
        if !self.seen.insert(id) {
            return Err("cyclic adjacency traversal".into());
        }
        let old = field(world, id, 8)? as u16;
        let flags = if self.seat { old | 0x100 } else { old & !0x100 };
        let callback = old != flags && function(world, p, id, 25)?;
        world
            .objects
            .get_mut(&id)
            .unwrap()
            .data
            .insert(8, flags as i16);
        self.advance = true;
        Ok(Some(Update {
            object: id,
            flags,
            callback,
        }))
    }
}
