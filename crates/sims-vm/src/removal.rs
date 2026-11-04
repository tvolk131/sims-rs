//! Native Remove Object selection (0x451b6a) and deferred queue (0x442d70).
use crate::{Location, ObjectId, Program, Status, ThreadId, Vm, World, current};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub flags: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pending {
    pub target: ObjectId,
    /// Original internal bit 0x1000 suppresses dependent-object resets.
    pub suppress_reset: bool,
}
pub fn selection(mode: i16, caller: ObjectId, stack: ObjectId) -> Result<ObjectId, String> {
    match mode {
        0 => Ok(caller),
        1 => Ok(stack),
        2 => Ok(0),
        _ => Err("Remove Object invalid mode (native error 0x09)".into()),
    }
}
/// Removal of an already queued object or descendant is a successful no-op.
pub fn already_queued(
    world: &World,
    pending: &[Pending],
    target: ObjectId,
) -> Result<bool, String> {
    let mut current = target;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        if !seen.insert(current) {
            return Err("cyclic removal containment".into());
        }
        if pending.iter().any(|p| p.target == current) {
            return Ok(true);
        }
        match world
            .objects
            .get(&current)
            .ok_or("missing removal object")?
            .container
        {
            Some((parent, _)) => current = parent,
            None => return Ok(false),
        }
    }
}
impl Vm {
    /// 0x425a69: copy the current engine's persistent person fields back to the
    /// existing neighborhood record. This is the in-memory record, not export
    /// to NBRS. Missing people/lookup slots are successful no-ops. The original
    /// revision helper 0x423e04 returns 9 even though the table also knows v10.
    pub fn writeback_person_neighbor(&mut self, object: ObjectId) -> Result<Option<i16>, String> {
        let Some(person) = self
            .world
            .objects
            .get(&object)
            .and_then(|o| o.person.as_ref())
        else {
            return Ok(None);
        };
        let id = *person.get(&31).ok_or("person neighbor identity absent")?;
        if id <= 0 {
            return Ok(None);
        }
        let directory = self
            .world
            .person
            .as_mut()
            .ok_or("person writeback neighborhood absent")?;
        let slot = match &directory.neighbor_slots {
            Some(slots) => slots.get(id as usize - 1).copied().flatten(),
            None => Some(id),
        };
        let Some(neighbor) = slot.and_then(|key| directory.neighbors.get_mut(&key)) else {
            return Ok(None);
        };
        let updates = crate::loading::persistent_person_fields(9)
            .into_iter()
            .map(|field| {
                if !neighbor.person.contains_key(&field) {
                    return Err("persistent neighbor destination absent");
                }
                Ok((
                    field,
                    *person
                        .get(&field)
                        .ok_or("persistent person source absent")?,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        neighbor.person_revision = 9;
        neighbor.person.extend(updates);
        Ok(Some(id))
    }

    pub fn finish_removal(
        &mut self,
        p: &Program,
        thread: ThreadId,
        request: u64,
    ) -> Result<(), String> {
        let t = self.threads.get(&thread).ok_or("missing removal thread")?;
        if !matches!(&t.status,Status::AwaitingRemoval(r) if r.id==request) {
            return Err("removal request mismatch".into());
        }
        let target = match &t.status {
            Status::AwaitingRemoval(r) => r.target,
            _ => unreachable!(),
        };
        // The wrapper writes back after mark (including an already-queued
        // mark), before either its self-yield or ordinary true return.
        self.writeback_person_neighbor(target)?;
        let t = &self.threads[&thread];
        if !t.check
            && matches!(&t.status,Status::AwaitingRemoval(r) if r.caller==r.target && r.flags&1==0)
        {
            // Native result 2 yields at this instruction. The registry deletes
            // the object at the end of its update pass; no true branch runs.
            self.threads.get_mut(&thread).unwrap().status = Status::Sleeping {
                wake_tick: self.tick.checked_add(1).ok_or("tick exhausted")?,
            };
            return Ok(());
        }
        let ins = current(p, t)?.clone();
        self.complete_host_primitive(p, thread, true, ins.on_true, ins.on_false)
    }

    /// Off-lot leaf branch used while loading global controllers, before the
    /// spatial host exists. These still join the native deferred deletion queue;
    /// an Init that requests deletion must finish synchronously, not erase its
    /// own interpreter. Placed objects and detach callbacks need the lot host.
    pub fn defer_offlot_removal(
        &mut self,
        p: &Program,
        pending: &mut Vec<Pending>,
        thread: ThreadId,
        request: &Request,
    ) -> Result<(), String> {
        if !matches!(&self.threads.get(&thread).ok_or("missing removal thread")?.status, Status::AwaitingRemoval(r) if r == request)
        {
            return Err("off-lot removal request mismatch".into());
        }
        if !already_queued(&self.world, pending, request.target)? {
            let o = &self.world.objects[&request.target];
            let d = o.definition.as_ref().ok_or("removal definition missing")?;
            let m = &p.modules[&d.source];
            if o.person.is_some()
                || o.multitile.is_some()
                || o.container.is_some()
                || o.slots.iter().any(Option::is_some)
                || self
                    .world
                    .positions
                    .get(&request.target)
                    .is_none_or(|pos| pos.x != -16 || pos.y != -16)
                || [9, 10, 26].iter().any(|&i| {
                    m.object_functions[&d.object]
                        .get(i)
                        .is_some_and(|f| f.action != 0)
                })
            {
                return Err(
                    "loaded controller deletion requires spatial detach/callback services".into(),
                );
            }
            let object = self.world.objects.get_mut(&request.target).unwrap();
            object
                .data
                .extend([(2, 0), (3, -1), (14, -1), (26, 0), (29, -5)]);
            *object.data.get_mut(&8).ok_or("missing detach flags")? &= !0x100;
            if let Some(flags) = &mut object.native_flags {
                *flags = (*flags & !0x1000) | if request.flags & 2 != 0 { 0x1000 } else { 0 };
            }
            pending.push(Pending {
                target: request.target,
                suppress_reset: request.flags & 2 != 0,
            });
        }
        self.finish_removal(p, thread, request.id)
    }
}
impl crate::action::Actor {
    /// The non-running-target branch of cXPerson's deletion notification,
    /// 0x45e791. Caller services returned cancellation callbacks before erasure.
    pub fn remove_object_references(&mut self, target: ObjectId) -> Result<Vec<ObjectId>, String> {
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.target == target || a.icon_owner == target)
            || self.empty_action_icon == target
        {
            return Err("deletion of an active action target requires native person reset".into());
        }
        let mut callbacks = vec![];
        self.queue.pending.retain(|a| {
            let remove = a.target == target || a.icon_owner == target;
            if remove && a.target != 0 {
                callbacks.push(a.target);
            }
            !remove
        });
        if self.previous.as_ref().is_some_and(|a| a.target == target) {
            self.previous = None;
        } else if let Some(a) = self.previous.as_mut().filter(|a| a.icon_owner == target) {
            a.icon_owner = 0;
        }
        Ok(callbacks)
    }
}
