//! Native deferred removal mark and physical detachment, including the immediate
//! child-removal service used by the end-of-tick ordinary-object destructor.
use crate::{Simulation, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{
    ObjectId, Program, Status, ThreadId, Vm,
    removal::{Pending, Request},
};
pub mod detach;
pub mod immediate;
mod unregister;

/// 0x442d70 resolves a fixed multipart secondary to its head before checking
/// the deferred queue. A nonzero master on an ordinary runtime class fails the
/// native RTTI cast and makes the mark a no-op. Function 8 keeps the selected
/// dynamic-group member as queue identity; this does not itself split the group.
pub fn queue_target(
    world: &sims_vm::World,
    p: &Program,
    selected: ObjectId,
) -> Result<Option<ObjectId>, String> {
    let Some(object) = world.objects.get(&selected) else {
        return Ok(None);
    };
    let definition = object
        .definition
        .as_ref()
        .ok_or("removal definition absent")?;
    let words = p
        .modules
        .get(&definition.source)
        .and_then(|m| m.object_definition_words.get(&definition.object))
        .ok_or("removal definition words absent")?;
    if words.get(10).copied().ok_or("removal master word absent")? == 0 {
        return Ok(Some(selected));
    }
    let Some(part) = &object.multitile else {
        return Ok(None);
    };
    let functions = p
        .modules
        .get(&definition.source)
        .and_then(|m| m.object_functions.get(&definition.object))
        .ok_or("removal functions absent")?;
    Ok(Some(if functions.get(8).is_some_and(|f| f.action != 0) {
        selected
    } else {
        part.head.unwrap_or(selected)
    }))
}

/// 0x442d70 captures each logical ancestor before checking the target itself.
/// Capture is observable: an unresolved sentinel predecessor normalizes its
/// physical slot even when the removal is already queued.
pub fn already_queued(
    world: &mut sims_vm::World,
    links: &crate::lot_links::Links,
    pending: &[Pending],
    target: ObjectId,
) -> Result<bool, String> {
    let mut current = target;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        if !seen.insert(current) {
            return Err("cyclic native removal containment".into());
        }
        let Some(parent) = links.capture(world, current)?.container else {
            break;
        };
        if pending.iter().any(|r| r.target == parent) {
            return Ok(true);
        }
        current = parent;
    }
    Ok(pending.iter().any(|r| r.target == target))
}

/// 0x44123f remembers lighting relevance and room before calling RemoveFromLot.
pub fn immediate_room(world: &sims_vm::World, target: ObjectId) -> Result<Option<i16>, String> {
    let data = &world
        .objects
        .get(&target)
        .ok_or("native immediate removal object absent")?
        .data;
    Ok(
        if *data.get(&7).ok_or("native removal word 7 absent")? != 0 {
            Some(*data.get(&29).ok_or("native removal room absent")?)
        } else {
            None
        },
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Origin {
    Primitive {
        thread: ThreadId,
        request: Request,
        /// Queue identity may be the group head while the VM request retains
        /// the selected secondary for writeback and self-removal yielding.
        target: ObjectId,
    },
    Child {
        parent: ObjectId,
        slot: usize,
        target: ObjectId,
        /// Immediate removal (0x44123f) captures word 7 and room 29 before
        /// physical detachment, then invalidates that room after destruction.
        after_delete_room: Option<i16>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    pub origin: Origin,
    pub placement: detach::Detach,
}
impl Execution {
    pub fn new(
        origin: Origin,
        world: &sims_vm::World,
        links: &crate::lot_links::Links,
    ) -> Result<Self, String> {
        let target = match &origin {
            Origin::Primitive { target, .. } => *target,
            Origin::Child { target, .. } => *target,
        };
        Ok(Self {
            origin,
            placement: detach::Detach::new(world, links, target)?,
        })
    }
    pub fn target(&self) -> ObjectId {
        self.placement.target()
    }

    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.placement.owns_callback(vm, p, thread, depth)
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let target = match &self.origin {
            Origin::Primitive {
                thread,
                request,
                target,
            } => {
                let t =
                    s.vm.threads
                        .get(thread)
                        .ok_or("native removal requester absent")?;
                let bound = std::iter::once(&t.status)
                    .chain(
                        t.callback
                            .iter()
                            .flat_map(|b| b.chain())
                            .map(|b| &b.resume_status),
                    )
                    .chain(
                        t.service
                            .iter()
                            .flat_map(|b| b.chain())
                            .map(|b| &b.resume_status),
                    )
                    .any(|status| matches!(status,Status::AwaitingRemoval(r) if r==request));
                if !bound
                    || s.removals.iter().any(|r| r.target == *target)
                    || queue_target(&s.vm.world, p, request.target)? != Some(*target)
                {
                    return Err("unbound native removal request".into());
                }
                *target
            }
            Origin::Child {
                parent,
                slot,
                target,
                ..
            } => {
                let d = s
                    .deletion
                    .last()
                    .ok_or("native child removal without destructor")?;
                if s.visit.is_some() && !s.owns_immediate_creation_deletion()
                    || d.target != *parent
                    || !d.notified
                    || d.next_slot != *slot
                    || d.slot_limit.is_none_or(|limit| *slot >= limit)
                {
                    return Err("unbound native child detachment".into());
                }
                *target
            }
        };
        self.placement.validate(s, p, target)
    }
}

impl Simulation {
    /// cXPerson shares base RemoveFromLot; its own Reset and storage destructor
    /// are dispatched later by the registry. Group roots include every member;
    /// contained groups still require their separate recursive service.
    pub(crate) fn native_removal_tree(&self, p: &Program, root: ObjectId) -> Result<(), String> {
        let members = if self
            .vm
            .world
            .objects
            .get(&root)
            .is_some_and(|o| o.multitile.is_some())
        {
            sims_vm::multitile::chain(&self.vm.world, root)?
        } else {
            vec![root]
        };
        let mut pending: Vec<_> = members.into_iter().map(|id| (id, true)).collect();
        let mut seen = std::collections::BTreeSet::new();
        while let Some((id, member)) = pending.pop() {
            if !seen.insert(id) {
                return Err("cyclic native removal subtree".into());
            }
            let o = self
                .vm
                .world
                .objects
                .get(&id)
                .ok_or("native removal object absent")?;
            let d = o
                .definition
                .as_ref()
                .ok_or("native removal definition absent")?;
            if p.modules[&d.source].object_definition_words[&d.object][9] == 9 && o.cursor.is_none()
                || o.multitile.is_some() && (o.person.is_some() || !member)
            {
                return Err(
                    "native special-object/contained-group removal override required".into(),
                );
            }
            pending.extend(o.slots.iter().flatten().map(|&id| (id, false)));
        }
        Ok(())
    }
    /// RemoveFromLot retains the animator; Find Location need not schedule a
    /// destructor afterward. A registered off-lot person can therefore keep
    /// its pose. Placement refresh never advances its simulation clock twice.
    pub(crate) fn retains_native_offlot_pose(&self, actor: ObjectId) -> bool {
        self.native_lot.as_ref().is_some_and(|n| {
            (self.unplaced.contains(&actor)
                && self.vm.object_threads.contains_key(&actor)
                && self
                    .vm
                    .world
                    .positions
                    .get(&actor)
                    .is_some_and(|q| q.x < 0 || q.y < 0))
                || n.removal.as_ref().is_some_and(|e| e.target() == actor)
                || self.removals.iter().any(|r| r.target == actor)
                || self.deletion.iter().any(|d| d.target == actor)
        })
    }
    pub(crate) fn drive_native_removal(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let Some(mut e) = self.native_lot.as_mut().and_then(|n| n.removal.take()) else {
            return Ok(true);
        };
        let result = e.placement.advance(self, p, remaining, step);
        if !result.as_ref().is_ok_and(|done| *done) {
            self.native_lot.as_mut().unwrap().removal = Some(e);
            return result;
        }
        match e.origin {
            Origin::Primitive {
                thread,
                request,
                target,
            } => {
                // 0x442d70 sets the reset-suppression flag and appends only
                // after virtual RemoveFromLot(1) has returned synchronously.
                if let Some(flags) =
                    &mut self.vm.world.objects.get_mut(&target).unwrap().native_flags
                {
                    *flags = (*flags & !0x1000) | if request.flags & 2 != 0 { 0x1000 } else { 0 };
                }
                self.removals.push(Pending {
                    target,
                    suppress_reset: request.flags & 2 != 0,
                });
                self.vm.finish_removal(p, thread, request.id)?;
                self.event(
                    &mut step.events,
                    thread,
                    format!(
                        "original physical removal #{} detached; queued end-of-tick deletion",
                        target
                    ),
                );
            }
            Origin::Child {
                parent,
                slot,
                target,
                after_delete_room,
            } => {
                let d = self
                    .deletion
                    .last_mut()
                    .ok_or("native child destructor disappeared")?;
                if d.target != parent || d.next_slot != slot {
                    return Err("native child removal cursor changed".into());
                }
                d.next_slot += 1;
                let mut deletion = crate::removal::Deletion::new(target);
                deletion.after_delete_room = after_delete_room;
                self.deletion.push(deletion);
            }
        }
        Ok(true)
    }
}
