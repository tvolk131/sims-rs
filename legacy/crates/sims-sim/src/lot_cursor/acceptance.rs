//!Buy placement permission435522/422de4. The group wrapper calls435522 on
//!each part; that base wrapper still dispatches virtual+54 (potentially422abb).
use super::{handle, id};
use crate::{Simulation, Step, lot_acceptance};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Query {
    pub object: u32,
    pub position: [i32; 2],
    pub level: i32,
    pub container: Option<(ObjectId, i32)>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    input: Query,
    cheat: bool,
    grouped: bool,
    origin: [i32; 2],
    level: i32,
    current: Option<u32>,
    seen: BTreeSet<u32>,
    member: Option<Query>,
    child: Option<lot_acceptance::live::Execution>,
    outcome: Option<bool>,
}
impl Check {
    pub fn new(s: &Simulation, input: Query, cheat: bool) -> Result<Self, String> {
        let selected = id(&s.vm, input.object)?;
        let o = &s.vm.world.objects[&selected];
        if o.cursor.is_some() {
            return Err("recursive cursor Buy acceptance requires its virtual host".into());
        }
        let mut origin = input.position;
        let mut level = input.level;
        let mut current = input.object;
        if let Some(part) = &o.multitile {
            if let Some(head) = part.head {
                origin[0] = origin[0].wrapping_sub(part.offset[1].wrapping_mul(16));
                origin[1] = origin[1].wrapping_sub(part.offset[0].wrapping_mul(16));
                level = level.wrapping_sub(part.offset[2]);
                current = handle(&s.vm, head)?;
            }
        }
        Ok(Self {
            input,
            cheat,
            grouped: o.multitile.is_some(),
            origin,
            level,
            current: Some(current),
            seen: BTreeSet::new(),
            member: None,
            child: None,
            outcome: None,
        })
    }
    pub fn binding(&self) -> Query {
        self.input
    }
    pub(crate) fn cheat(&self) -> bool {
        self.cheat
    }
    pub fn outcome(&self) -> Option<bool> {
        self.outcome
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.child
            .as_ref()
            .is_some_and(|c| c.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        id(vm, self.input.object)?;
        if let Some(current) = self.current {
            id(vm, current)?;
        }
        if self.outcome.is_some() && (self.member.is_some() || self.child.is_some())
            || self.member.is_some() != self.child.is_some()
        {
            return Err("invalid Buy acceptance continuation".into());
        }
        if let Some(q) = self.member {
            let child = self.child.as_ref().unwrap();
            child.validate(vm, p)?;
            let object = id(vm, q.object)?;
            if self.current != Some(q.object)
                || !self.seen.contains(&q.object)
                || child.binding().0 != object
                || child.binding().2 != q.level
                || child.container_binding() != q.container
            {
                return Err("Buy acceptance child binding differs".into());
            }
            if q.container.is_none() && child.binding().1 != q.position {
                return Err("Buy acceptance position differs".into());
            }
        }
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        while *remaining > 0 && self.outcome.is_none() {
            if let Some(child) = &mut self.child {
                if !s.drive_native_acceptance(p, child, remaining, step)? {
                    return Ok(false);
                }
                let mut accepted = child.outcome().ok_or("Buy +54 outcome absent")?.accepted;
                let q = self.member.ok_or("Buy acceptance member absent")?;
                if accepted {
                    //Test the caller's point, even when+54 builds a contained
                    //descriptor from the parent's physical point instead.
                    accepted = q.position[0] & 15 == 8 && q.position[1] & 15 == 8;
                    if accepted
                        && !self.cheat
                        && let Some((parent, slot)) = q.container
                    {
                        let slots = lot_acceptance::contained::slots(&s.vm.world, p, parent)?;
                        if usize::try_from(slot)
                            .ok()
                            .and_then(|n| slots.get(n))
                            .is_some_and(|slot| (5..8).contains(&slot.height))
                        {
                            s.native_lot.as_mut().unwrap().diagnostics.error = 11;
                            accepted = false;
                        }
                    }
                }
                self.child = None;
                self.member = None;
                if !accepted {
                    self.outcome = Some(false);
                    break;
                }
                if self.grouped {
                    let object = id(&s.vm, q.object)?;
                    self.current = s.vm.world.objects[&object]
                        .multitile
                        .as_ref()
                        .ok_or("Buy group member disappeared")?
                        .next
                        .map(|id| handle(&s.vm, id))
                        .transpose()?;
                } else {
                    self.current = None;
                }
                continue;
            }
            let Some(current) = self.current else {
                self.outcome = Some(true);
                break;
            };
            if !self.seen.insert(current) {
                return Err("cyclic Buy acceptance group".into());
            }
            let object = id(&s.vm, current)?;
            let q = if self.grouped {
                let offset = s.vm.world.objects[&object]
                    .multitile
                    .as_ref()
                    .ok_or("Buy group storage absent")?
                    .offset;
                Query {
                    object: current,
                    position: [
                        self.origin[0].wrapping_add(offset[1].wrapping_mul(16)),
                        self.origin[1].wrapping_add(offset[0].wrapping_mul(16)),
                    ],
                    level: self.level.wrapping_add(offset[2]),
                    container: None,
                }
            } else {
                self.input
            };
            *remaining -= 1;
            let native = s.native_lot.as_mut().ok_or("Buy acceptance lot absent")?;
            native.diagnostics = lot_acceptance::person::Diagnostics {
                error: 0,
                blocker: None,
            };
            let size = native.placement.links.size;
            if size < 2 {
                return Err("Buy acceptance lot size invalid".into());
            }
            let [column, row] = [(q.position[1] >> 4) as i8, (q.position[0] >> 4) as i8];
            let outside =
                column < 1 || row < 1 || column as usize >= size - 1 || row as usize >= size - 1;
            if outside
                || !self.cheat
                    && native
                        .placement
                        .flags
                        .first()
                        .and_then(|f| f.get(column as usize * size + row as usize))
                        .ok_or("Buy ground flags absent")?
                        & 0x20
                        != 0
            {
                native.diagnostics.error = 1;
                self.outcome = Some(false);
                break;
            }
            self.child = Some(if let Some((parent, slot)) = q.container {
                lot_acceptance::live::Execution::new_contained(s, object, parent, slot, q.level)?
            } else {
                lot_acceptance::live::Execution::new(s, object, q.position, q.level)?
            });
            self.member = Some(q);
        }
        Ok(self.outcome.is_some())
    }
}
