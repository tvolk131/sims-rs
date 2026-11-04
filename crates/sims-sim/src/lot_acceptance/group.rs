//! Native 0x422abb: transform the selected member's destination to the group
//! origin, check every sibling, then publish the accumulated placement error.
use super::Outcome;
use crate::lot_links::Links;
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, World};
use std::collections::BTreeSet;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Query {
    pub object: ObjectId,
    pub position: [i32; 2],
    pub level: i32,
    pub slot: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    selected: ObjectId,
    origin: [i32; 2],
    selected_offset: [i32; 3],
    contained: bool,
    slot: i32,
    level: i32,
    current: Option<ObjectId>,
    seen: BTreeSet<ObjectId>,
    pending: Option<Query>,
    error: i32,
    last_error: i32,
    height: Option<u8>,
    outcome: Option<Outcome>,
}
impl Check {
    pub fn begin(
        w: &World,
        links: &Links,
        selected: ObjectId,
        position: [i32; 2],
        level: i32,
        contained: bool,
        slot: i32,
        error: i32,
    ) -> Result<Self, String> {
        let chain = sims_vm::multitile::chain(w, selected)?;
        let offset = w.objects[&selected].multitile.as_ref().unwrap().offset;
        let origin = [
            position[0].wrapping_sub(offset[1].wrapping_mul(16)),
            position[1].wrapping_sub(offset[0].wrapping_mul(16)),
        ];
        let outcome = if contained {
            Some(Outcome {
                accepted: false,
                error: 16,
            })
        } else if !links.on_lot(origin) {
            Some(Outcome {
                accepted: false,
                error: 1,
            })
        } else {
            None
        };
        Ok(Self {
            selected,
            origin,
            selected_offset: offset,
            contained,
            slot,
            level: level.wrapping_sub(offset[2]),
            current: Some(chain[0]),
            seen: BTreeSet::new(),
            pending: None,
            error: 0,
            last_error: error,
            height: None,
            outcome,
        })
    }
    pub fn binding(&self) -> (ObjectId, [i32; 2], i32) {
        (
            self.selected,
            [
                self.origin[0].wrapping_add(self.selected_offset[1].wrapping_mul(16)),
                self.origin[1].wrapping_add(self.selected_offset[0].wrapping_mul(16)),
            ],
            self.level.wrapping_add(self.selected_offset[2]),
        )
    }
    pub fn contained(&self) -> bool {
        self.contained
    }
    pub fn pending(&self) -> Option<Query> {
        self.pending
    }
    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }
    pub fn next(&mut self, w: &World) -> Result<Option<Query>, String> {
        if self.outcome.is_some() {
            return Ok(None);
        }
        if let Some(q) = self.pending {
            return Ok(Some(q));
        }
        let Some(id) = self.current else {
            self.outcome = Some(Outcome {
                accepted: self.error == 0,
                error: if self.error == 0 {
                    self.last_error
                } else {
                    self.error
                },
            });
            return Ok(None);
        };
        if !self.seen.insert(id) {
            return Err("cyclic multipart acceptance traversal".into());
        }
        let offset = w
            .objects
            .get(&id)
            .and_then(|o| o.multitile.as_ref())
            .ok_or("multipart acceptance part absent")?
            .offset;
        let q = Query {
            object: id,
            position: [
                self.origin[0].wrapping_add(offset[1].wrapping_mul(16)),
                self.origin[1].wrapping_add(offset[0].wrapping_mul(16)),
            ],
            level: self.level.wrapping_add(offset[2]),
            slot: self.slot,
        };
        self.pending = Some(q);
        Ok(Some(q))
    }
    pub fn resume(
        &mut self,
        w: &World,
        size: usize,
        heights: &[[u8; 4]],
        q: Query,
        result: Outcome,
    ) -> Result<(), String> {
        if self.pending != Some(q) || self.outcome.is_some() {
            return Err("multipart acceptance reply mismatch".into());
        }
        if !result.accepted {
            if self.error == 0 || [30, 31].contains(&result.error) {
                self.error = result.error;
            }
            if result.error != 1
                && w.objects[&self.selected]
                    .data
                    .get(&42)
                    .ok_or("group placement flags absent")?
                    & 0x100
                    == 0
            {
                // Only failed base checks take this branch. Native compares the
                // low byte of the height dword against the first such failure.
                let col = (q.position[1] >> 4) as i8;
                let row = (q.position[0] >> 4) as i8;
                if col < 0
                    || row < 0
                    || col as usize >= size
                    || row as usize >= size
                    || heights.len() != size * size
                {
                    return Err("multipart acceptance height query outside arrays".into());
                }
                let height = heights[col as usize * size + row as usize][0];
                if let Some(old) = self.height {
                    if old != height && self.error == 0 {
                        self.error = 17;
                    }
                } else {
                    self.height = Some(height);
                }
            }
        }
        self.last_error = result.error;
        self.current = w
            .objects
            .get(&q.object)
            .and_then(|o| o.multitile.as_ref())
            .ok_or("multipart acceptance part removed during callback")?
            .next;
        self.pending = None;
        Ok(())
    }
    pub fn validate(&self, w: &World) -> Result<(), String> {
        if w.objects
            .get(&self.selected)
            .is_none_or(|o| o.multitile.is_none())
        {
            return Err("multipart acceptance selected object absent".into());
        }
        if self.current.is_some_and(|id| !w.objects.contains_key(&id))
            || self.outcome.is_some() && self.pending.is_some()
            || self
                .pending
                .is_some_and(|q| Some(q.object) != self.current || !self.seen.contains(&q.object))
        {
            return Err("invalid multipart acceptance continuation".into());
        }
        Ok(())
    }
}
