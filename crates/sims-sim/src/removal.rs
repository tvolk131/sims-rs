//! Deferred ordinary-object deletion. The original registry drains removals
//! after its object-update pass (0x4412d1), never at a diagnostic budget boundary.
use crate::{Event, Simulation, Step};
use serde::{Deserialize, Serialize};
pub mod base;
pub mod character;
pub mod cursor;
pub mod entry;
pub mod graphic;
#[cfg(feature = "private-fixtures")]
#[cfg(test)]
mod live_tests;
pub mod person;
/// 0x440eeb marks the whole chain, then captures a member's next pointer before
/// its recursive force-part call. This wrapper outlives its original head.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    pub marked: Vec<ObjectId>,
    pub next: Option<ObjectId>,
    pub active: Option<ObjectId>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Deletion {
    pub target: ObjectId,
    pub notified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<entry::Execution>,
    pub next_slot: usize,
    /// 0x440eeb captures the slot-vector length before visiting its children.
    pub slot_limit: Option<usize>,
    pub after_delete_room: Option<i16>,
    pub person_reset: Option<person::Execution>,
    pub base_reset: Option<base::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<cursor::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_definition: Option<sims_vm::Definition>,
    pub force_part: bool,
    pub group: Option<Group>,
    #[serde(default)]
    pub storage_unlinked: bool,
    #[serde(default)]
    pub assets_released: bool,
}
impl Deletion {
    pub(crate) fn new(target: ObjectId) -> Self {
        Self {
            target,
            notified: false,
            entry: None,
            next_slot: 0,
            slot_limit: None,
            after_delete_room: None,
            person_reset: None,
            base_reset: None,
            cursor: None,
            entry_definition: None,
            force_part: false,
            group: None,
            storage_unlinked: false,
            assets_released: false,
        }
    }
}
use sims_vm::{
    ObjectId, Program, Status, ThreadId,
    removal::{Pending, Request},
};
impl Simulation {
    pub(crate) fn empty_detach_callbacks(&self, p: &Program, id: ObjectId) -> Result<(), String> {
        let o = &self.vm.world.objects[&id];
        let d = o
            .definition
            .as_ref()
            .ok_or("removal requires an object definition")?;
        let m = &p.modules[&d.source];
        if o.person.is_some()
            || (m.object_definition_words[&d.object][0x14 / 2] != 0
                && sims_vm::multitile::chain(&self.vm.world, id)?.len() != 1)
        {
            return Err("removal of people or multi-part objects is unsupported".into());
        }
        for function in [9, 10, 26] {
            if m.object_functions[&d.object]
                .get(function)
                .is_some_and(|f| f.action != 0)
            {
                return Err(format!(
                    "nonempty removal placement callback {function} unsupported"
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn empty_removal_callbacks(&self, p: &Program, id: ObjectId) -> Result<(), String> {
        self.empty_detach_callbacks(p, id)?;
        for child in self.vm.world.objects[&id].slots.iter().flatten() {
            self.empty_detach_callbacks(p, *child)?;
        }
        Ok(())
    }
    /// Ordinary RemoveFromLot (0x433125) unlinks only the root, then 0x433e50
    /// propagates the off-lot position through all physical containment slots.
    pub(crate) fn detach_tree(&mut self, root: ObjectId) -> Result<(), String> {
        let mut pending = vec![root];
        let mut descendants = std::collections::BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !descendants.insert(id) {
                return Err("cyclic detach containment".into());
            }
            let object = self
                .vm
                .world
                .objects
                .get(&id)
                .ok_or("missing detach child")?;
            if object.person.is_some()
                || object
                    .multitile
                    .as_ref()
                    .is_some_and(|m| m.head.is_some() || m.next.is_some())
            {
                return Err(
                    "detaching contained people or multi-part objects is unsupported".into(),
                );
            }
            pending.extend(object.slots.iter().flatten().copied());
        }
        if let Some((parent, slot)) = self.vm.world.objects[&root].container {
            self.vm.world.objects.get_mut(&parent).unwrap().slots[slot as usize] = None;
        }
        let object = self.vm.world.objects.get_mut(&root).unwrap();
        object.container = None;
        object.data.extend([(2, 0), (3, -1), (14, -1), (26, 0)]);
        *object.data.get_mut(&8).ok_or("missing detach flags")? &= !0x100;
        for id in descendants {
            self.vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .data
                .insert(29, -5);
            self.bodies.remove(&id);
            self.unplaced.insert(id);
            if let Some(a) = self
                .vm
                .world
                .person
                .as_mut()
                .and_then(|p| p.autonomy.as_mut())
            {
                if let Some(set) = &mut a.advertised {
                    set.remove(&id);
                }
                a.placements.remove(&id);
                if let Some(set) = &mut a.full_scan {
                    set.remove(&id);
                }
            }
        }
        self.sync_spatial_world();
        Ok(())
    }
    pub(crate) fn service_removal(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: &Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if self.native_lot.is_some() {
            let Some(target) = crate::lot_removal::queue_target(&self.vm.world, p, r.target)?
            else {
                self.vm.finish_removal(p, thread, r.id)?;
                return Ok(true);
            };
            if crate::lot_removal::already_queued(
                &mut self.vm.world,
                &self.native_lot.as_ref().unwrap().placement.links,
                &self.removals,
                target,
            )? {
                self.vm.finish_removal(p, thread, r.id)?;
                return Ok(true);
            }
            self.native_removal_tree(p, target)?;
            if self.native_lot.as_ref().unwrap().removal.is_some() {
                return Err("overlapping native physical removals".into());
            }
            self.native_lot.as_mut().unwrap().removal = Some(crate::lot_removal::Execution::new(
                crate::lot_removal::Origin::Primitive {
                    thread,
                    request: r.clone(),
                    target,
                },
                &self.vm.world,
                &self.native_lot.as_ref().unwrap().placement.links,
            )?);
            return self.drive_native_removal(p, remaining, step);
        }
        if !sims_vm::removal::already_queued(&self.vm.world, &self.removals, r.target)? {
            self.empty_removal_callbacks(p, r.target)?;
            self.detach_tree(r.target)?;
            if let Some(flags) = self
                .vm
                .world
                .objects
                .get_mut(&r.target)
                .unwrap()
                .native_flags
                .as_mut()
            {
                *flags = (*flags & !0x1000) | if r.flags & 2 != 0 { 0x1000 } else { 0 };
            }
            self.removals.push(Pending {
                target: r.target,
                suppress_reset: r.flags & 2 != 0,
            });
            self.event(
                &mut step.events,
                thread,
                format!("detached #{}; queued end-of-tick deletion", r.target),
            );
        }
        self.vm.finish_removal(p, thread, r.id)?;
        self.sync_carry(p)?;
        Ok(true)
    }

    /// Function 3 runs synchronously inside native Reset. Remove Object still
    /// marks through 442d70 and writes the neighbor back through 425a69, even
    /// when the object is already queued. Its synchronous return mode is owned
    /// by the VM callback; do not yield or recursively destroy the requester.
    fn drive_deletion_callback(
        &mut self,
        p: &Program,
        thread: ThreadId,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        while self.vm.threads[&thread].callback.is_some() {
            if *remaining == 0 {
                return Ok(false);
            }
            match self.vm.threads[&thread].status.clone() {
                Status::Ready => {
                    let slice = self.vm.run_slice(p, thread, 1)?;
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                }
                Status::AwaitingRemoval(request) => {
                    if !self.service_removal(p, thread, &request, remaining, step)? {
                        return Ok(false);
                    }
                }
                status => {
                    return Err(format!("deletion Reset callback host required: {status:?}"));
                }
            }
        }
        Ok(true)
    }
    pub(crate) fn flush_removals(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        while !self.removals.is_empty() {
            if self.deletion.is_empty() {
                let target = self.removals[0].target;
                if !self.vm.world.objects.contains_key(&target) {
                    self.removals.remove(0);
                    continue;
                }
                self.deletion.push(Deletion::new(target));
            }
            if !self.drive_deletion_stack(p, remaining, step, true)? {
                return Ok(false);
            }
            self.removals.remove(0);
        }
        Ok(true)
    }
    /// Shared 0x440eeb traversal, used both by the tick-end queue and the
    /// synchronous 0x44123f cleanup of a rejected Create Object instance.
    pub(crate) fn drive_deletion_stack(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
        deferred: bool,
    ) -> Result<bool, String> {
        self.drive_deletion_stack_until(p, remaining, step, deferred, 0)
    }
    pub(crate) fn drive_deletion_stack_until(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
        deferred: bool,
        depth: usize,
    ) -> Result<bool, String> {
        while self.deletion.len() > depth {
            //Resume a nested44123f through its retained cursor caller. Draining
            //the child frames here would skip the caller's return transition at
            //a budget boundary and leave its saved deletion owner incomplete.
            if let Some(index) = (depth..self.deletion.len().saturating_sub(1)).find(|&i| {
                self.deletion[i]
                    .cursor
                    .as_ref()
                    .is_some_and(|e| !e.complete())
            }) {
                if !self.drive_deletion_cursor_at(p, remaining, step, index)? {
                    return Ok(false);
                }
                continue;
            }
            let mut cursor = self.deletion.last().unwrap().clone();
            let id = cursor.target;
            if self.native_lot.is_some() && !cursor.force_part {
                if cursor.group.is_none()
                    && self
                        .vm
                        .world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.multitile.is_some())
                {
                    let marked = sims_vm::multitile::chain(&self.vm.world, id)?;
                    // 440eeb marks all members before the first recursive
                    // deletion; sibling callbacks observe these live bits.
                    for member in &marked {
                        if let Some(flags) =
                            &mut self.vm.world.objects.get_mut(member).unwrap().native_flags
                        {
                            *flags |= 0x40;
                        }
                    }
                    cursor.group = Some(Group {
                        next: marked.first().copied(),
                        marked,
                        active: None,
                    });
                    self.deletion.last_mut().unwrap().group = cursor.group.clone();
                }
                if let Some(group) = &cursor.group {
                    if let Some(next) = group.next {
                        let following = self
                            .vm
                            .world
                            .objects
                            .get(&next)
                            .and_then(|o| o.multitile.as_ref())
                            .ok_or("multipart deletion next member disappeared")?
                            .next;
                        let group = self.deletion.last_mut().unwrap().group.as_mut().unwrap();
                        group.next = following;
                        group.active = Some(next);
                        let mut part = Deletion::new(next);
                        part.force_part = true;
                        self.deletion.push(part);
                    } else {
                        let completed = self.deletion.pop().unwrap();
                        if let Some(room) = completed.after_delete_room {
                            self.vm.refresh_room_event(0xf3, i32::from(room))?;
                        }
                    }
                    continue;
                }
            }
            if cursor.storage_unlinked {
                if !cursor.assets_released {
                    return Ok(false);
                }
                let completed = self.deletion.pop().unwrap();
                if let Some(room) = completed.after_delete_room {
                    self.vm.refresh_room_event(0xf3, i32::from(room))?;
                }
                continue;
            }
            if !cursor.notified {
                if self.native_lot.is_some() {
                    self.capture_deletion_cursor(p)?;
                }
                if self.native_lot.is_some() && self.vm.world.objects[&id].native_flags.is_some() {
                    if !self.drive_deletion_entry(p, remaining, step)? {
                        return Ok(false);
                    }
                } else {
                    self.notify_deletion(p, id)?;
                }
                self.deletion.last_mut().unwrap().notified = true;
            }
            if !self.drive_cancellation(p, remaining, step)? {
                return Ok(false);
            }
            if self.native_lot.is_some() && !self.drive_deletion_person_reset(p, remaining, step)? {
                return Ok(false);
            }
            if self.native_lot.is_some() && !self.drive_deletion_base_reset(p, remaining, step)? {
                return Ok(false);
            }
            if self.native_lot.is_some()
                && !self.drive_deletion_cursor_at(p, remaining, step, self.deletion.len() - 1)?
            {
                return Ok(false);
            }
            let cursor = self.deletion.last_mut().unwrap();
            let limit = *cursor
                .slot_limit
                .get_or_insert(self.vm.world.objects[&id].slots.len());
            let mut child = None;
            while cursor.next_slot < limit {
                let next = *self.vm.world.objects[&id]
                    .slots
                    .get(cursor.next_slot)
                    .ok_or("deletion callback shrank native slot vector")?;
                if let Some(next) = next {
                    child = Some(next);
                    break;
                }
                cursor.next_slot += 1;
            }
            if let Some(child) = child {
                if self.native_lot.is_some() {
                    let slot = self.deletion.last().unwrap().next_slot;
                    self.native_removal_tree(p, child)?;
                    self.unregister_immediate_removal(p, child)?;
                    let after_delete_room =
                        crate::lot_removal::immediate_room(&self.vm.world, child)?;
                    self.native_lot.as_mut().unwrap().removal =
                        Some(crate::lot_removal::Execution::new(
                            crate::lot_removal::Origin::Child {
                                parent: id,
                                slot,
                                target: child,
                                after_delete_room,
                            },
                            &self.vm.world,
                            &self.native_lot.as_ref().unwrap().placement.links,
                        )?);
                    if !self.drive_native_removal(p, remaining, step)? {
                        return Ok(false);
                    }
                    continue;
                }
                self.empty_removal_callbacks(p, child)?;
                self.detach_tree(child)?;
                self.deletion.last_mut().unwrap().next_slot += 1;
                self.deletion.push(Deletion::new(child));
                continue;
            }
            self.free_removed_object(p, id, &mut step.events, deferred)?;
            if self.retired_characters.contains_key(&id) || self.retired_graphics.contains_key(&id)
            {
                self.deletion.last_mut().unwrap().storage_unlinked = true;
                return Ok(false);
            }
            let completed = self.deletion.pop().unwrap();
            if let Some(room) = completed.after_delete_room {
                self.vm.refresh_room_event(0xf3, i32::from(room))?;
            }
        }
        Ok(true)
    }
    fn notify_deletion(&mut self, _p: &Program, id: ObjectId) -> Result<(), String> {
        if self.vm.world.ui_owner.dialog_owners.contains(&id) {
            return Err("deletion with live UI dialogs requires dialog disposal".into());
        }
        let own = self.vm.object_threads.get(&id).copied();
        // Native deletion resets dependent people and unwinds their frames.
        // A completed Food action needs only the reference cleanup below.
        if self.vm.threads.iter().any(|(&thread, t)| {
            Some(thread) != own
                && (t
                    .stack
                    .iter()
                    .any(|f| f.caller == id || f.callee == id || f.action_target == Some(id))
                    || t.dispatch
                        .as_ref()
                        .is_some_and(|d| d.action.target == id || d.action.icon_owner == id)
                    || t.autonomy.is_some())
        }) {
            return Err(
                "deletion while another execution depends on the object requires native reset"
                    .into(),
            );
        }
        // Publish required function-4 callbacks before children are destroyed.
        if let Some(flags) = &mut self.vm.world.objects.get_mut(&id).unwrap().native_flags {
            *flags |= 0x40;
        }
        self.vm.world.ui_owner.release_if_owner(id)?;
        // Their frames run on the still-live target and may span host budgets.
        if let Some(actions) = &mut self.vm.world.actions {
            for (&actor, state) in &mut actions.actors {
                for target in state.remove_object_references(id)? {
                    actions.callbacks.push_back(sims_vm::action::Callback {
                        object: target,
                        function: 4,
                        stack_object: actor,
                    });
                }
            }
        }
        Ok(())
    }
    fn free_removed_object(
        &mut self,
        p: &Program,
        id: ObjectId,
        events: &mut Vec<Event>,
        deferred: bool,
    ) -> Result<(), String> {
        // 44123f unregisters its target before detachment. Deferred 440eeb
        // does not have that prelude. Keep unsupported pooled/deferred deletion
        // (or a surviving duplicate reference) explicit before unlinking storage.
        if self.native_indicator_pools.active.contains(&Some(id))
            || self.native_indicator_pools.spare.contains(&Some(id))
        {
            return Err(
                "native pooled-indicator deletion requires reference-lifetime service".into(),
            );
        }
        if self.vm.world.indicator_queues.dates.contains(&Some(id))
            || self.vm.world.indicator_queues.stars.contains(&Some(id))
        {
            return Err("native deletion retains a pending indicator refresh reference".into());
        }
        if self.vm.world.objects[&id].cursor.is_some() {
            let native = self
                .native_lot
                .as_mut()
                .ok_or("cursor storage removal lot absent")?;
            if native.placement.links.next.get(&id) != Some(&0) {
                return Err("cursor storage unlink retains a physical successor".into());
            }
            native.placement.links.unlink(&mut self.vm.world, id)?;
        }
        if let Some(native) = &self.native_lot {
            if !self.unplaced.contains(&id) && self.vm.world.objects[&id].cursor.is_none()
                || native.placement.links.next.get(&id) != Some(&0)
                || native
                    .placement
                    .links
                    .heads
                    .iter()
                    .flatten()
                    .any(|&head| head == id)
                || native.placement.links.next.values().any(|&next| next == id)
                || self
                    .vm
                    .world
                    .objects
                    .values()
                    .any(|o| o.slots.contains(&Some(id)))
                || (!self.vm.world.registry_ui_disabled
                    && [native.indicators.primary, native.indicators.secondary].contains(&Some(id)))
            {
                return Err(
                    "native deletion requires detached physical links and indicator release".into(),
                );
            }
        }
        if self.retired_characters.contains_key(&id) || self.retired_graphics.contains_key(&id) {
            return Err("overlapping retired graphic identity".into());
        }
        let retired_graphic = if self.ordinary_graphic_required(p, id)? {
            Some(graphic::Retired {
                definition: self.vm.world.objects[&id].definition.clone().unwrap(),
                registry_lifetime: self
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .and_then(|s| s.handle(id)),
                tick: self.vm.tick,
                asset_service_started: false,
            })
        } else {
            None
        };
        let retired = if let Some(resident) = self.characters.get(&id) {
            Some(character::Retired {
                resident: resident.clone(),
                person: self.vm.world.objects[&id]
                    .person
                    .clone()
                    .ok_or("native character person absent")?,
                tick: self.vm.tick,
                asset_service_started: false,
            })
        } else {
            None
        };
        let own = self.vm.object_threads.get(&id).copied();
        if self.native_lot.is_some() && self.vm.world.objects[&id].multitile.is_some() {
            sims_vm::multitile::unlink(&mut self.vm.world, id)?;
        }
        if let Some(thread) = own {
            self.release(thread);
            self.vm.threads.remove(&thread);
            self.visited.remove(&thread);
            self.vm.object_threads.remove(&id);
        }
        self.vm.lifecycles.remove(&id);
        self.vm.world.unregister_scheduler_object(id)?;
        self.vm.world.objects.remove(&id);
        if let Some(native) = &mut self.native_lot {
            native.placement.links.next.remove(&id);
            native.placement.bounds.remove(&id);
            native.snap_offsets.remove(&id);
            native.route_paths.remove(&id);
            native.membership.0.remove(&id);
            if native.selected_person == Some(id) {
                native.selected_person = None;
            }
        }
        // 0x44383c erases the person vector entry before the cXPerson storage
        // destructor releases its animator, queues and frame arena.
        if let Some(household) = &mut self.household {
            household.people.remove(&id);
        }
        if let Some(actions) = &mut self.vm.world.actions {
            actions.actors.remove(&id);
        }
        self.person_factories.remove(&id);
        self.animations.remove(&id);
        self.characters.remove(&id);
        if let Some(retired) = retired {
            self.retired_characters.insert(id, retired);
        }
        if let Some(retired) = retired_graphic {
            self.retired_graphics.insert(id, retired);
        }
        self.carry.remove(&id);
        self.turns.remove(&id);
        self.portal_cache.remove(&id);
        self.interactions.remove(&id);
        self.bodies.remove(&id);
        self.vm.world.positions.remove(&id);
        self.backgrounds.remove(&id);
        self.unplaced.remove(&id);
        self.reservations.remove(&id);
        if let Some(person) = &mut self.vm.world.person {
            person.object_order.retain(|&o| o != id);
            person.person_order.retain(|&o| o != id);
        }
        // 44383c removes every fire registration after the person/multipart
        // vectors and before the resource-instance cache. Do not leave stale
        // memberships to attach to a later object that reuses this script ID.
        self.vm.world.fires.remove_object(id);
        if let Some(person) = &mut self.vm.world.person {
            for order in person.resource_instances.values_mut() {
                order.retain(|&o| o != id);
            }
            for chain in &mut person.tile_chains {
                chain.retain(|&o| o != id);
            }
            person.tile_chains.retain(|c| !c.is_empty());
            if let Some(a) = &mut person.autonomy {
                a.placements.remove(&id);
                if let Some(set) = &mut a.advertised {
                    set.remove(&id);
                }
                if let Some(set) = &mut a.full_scan {
                    set.remove(&id);
                }
            }
        }
        self.vm.world.invalidate_registry_order();
        if let Some(refresh) = &mut self.vm.world.refresh {
            refresh.draw.remove(&id);
            refresh.invalidated.remove(&id);
        }
        self.event(
            events,
            own.unwrap_or(0),
            if deferred {
                format!("deleted #{id} after lot object updates")
            } else {
                format!("deleted #{id} during synchronous factory cleanup")
            },
        );
        Ok(())
    }
    pub(crate) fn validate_removals(&self, p: &Program) -> Result<(), String> {
        if let Some(e) = self.native_lot.as_ref().and_then(|n| n.removal.as_ref()) {
            e.validate(self, p)?;
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in &self.removals {
            let unlinked = self
                .deletion
                .iter()
                .any(|d| d.target == r.target && d.storage_unlinked);
            let group = self
                .deletion
                .iter()
                .any(|d| d.target == r.target && d.group.is_some());
            if r.target <= 0
                || !seen.insert(r.target)
                || self.vm.world.objects.contains_key(&r.target)
                    && !self.unplaced.contains(&r.target)
                    && !self.deletion.iter().any(|d| {
                        d.target == r.target && d.cursor.as_ref().is_some_and(|e| e.may_be_onlot())
                    })
                    && !(group && self.vm.world.objects[&r.target].cursor.is_some())
                    && !unlinked
            {
                return Err("invalid deferred deletion queue".into());
            }
            // A group wrapper retains its queue identity after storage unlink.
            // Another queued identity may already have been deleted recursively;
            // the native queue drain skips that now-missing registry entry.
            if unlinked || group || !self.vm.world.objects.contains_key(&r.target) {
                continue;
            } else if self.native_lot.is_some() {
                self.native_removal_tree(p, r.target)?;
            } else {
                self.empty_removal_callbacks(p, r.target)?;
            }
        }
        if !self.deletion.is_empty() {
            if !self.owns_immediate_creation_deletion()
                && (self.visit.is_some()
                    || self.removals.first().map(|r| r.target) != Some(self.deletion[0].target)
                    || self.deletion[0].after_delete_room.is_some())
            {
                return Err("deletion continuation outside end-of-tick pass".into());
            }
            let mut deleting = std::collections::BTreeSet::new();
            for (index, d) in self.deletion.iter().enumerate() {
                if let Some(g) = &d.group {
                    let unique: std::collections::BTreeSet<_> = g.marked.iter().copied().collect();
                    let child = self.deletion.get(index + 1);
                    if self.native_lot.is_none()
                        || d.force_part
                        || d.notified
                        || d.entry.is_some()
                        || d.next_slot != 0
                        || d.slot_limit.is_some()
                        || d.person_reset.is_some()
                        || d.base_reset.is_some()
                        || d.cursor.is_some()
                        || d.entry_definition.is_some()
                        || d.storage_unlinked
                        || d.assets_released
                        || g.marked.is_empty()
                        || unique.len() != g.marked.len()
                        || !unique.contains(&d.target)
                        || unique.iter().any(|&id| id <= 0)
                        || g.active.is_none()
                        || child.is_none_or(|c| Some(c.target) != g.active || !c.force_part)
                        || g.next.is_some_and(|id| {
                            Some(id) == g.active
                                || self
                                    .vm
                                    .world
                                    .objects
                                    .get(&id)
                                    .is_none_or(|o| o.multitile.is_none())
                        })
                    {
                        return Err("invalid multipart deletion traversal".into());
                    }
                    continue;
                }
                if d.force_part
                    && index
                        .checked_sub(1)
                        .and_then(|i| self.deletion[i].group.as_ref())
                        .is_none_or(|g| g.active != Some(d.target))
                {
                    return Err("unbound forced multipart deletion".into());
                }
                if d.storage_unlinked {
                    if !deleting.insert(d.target)
                        || self.vm.world.objects.contains_key(&d.target)
                        || self.unplaced.contains(&d.target)
                        || !d.notified
                        || d.entry.as_ref().is_some_and(|e| !e.retired(d.target))
                        || d.slot_limit != Some(d.next_slot)
                        || d.cursor.as_ref().is_some_and(|e| !e.complete())
                        || (self.retired_characters.contains_key(&d.target)
                            || self.retired_graphics.contains_key(&d.target))
                            == d.assets_released
                        || !(d.base_reset.is_none()
                            && d.person_reset.as_ref().is_some_and(|r| {
                                r.complete && r.pending.is_none() && !r.asset_service_started
                            })
                            || d.person_reset.is_none()
                                && d.base_reset.as_ref().is_some_and(base::Execution::complete))
                        || self.retired_characters.contains_key(&d.target)
                            && d.person_reset.is_none()
                        || self.retired_graphics.contains_key(&d.target) && d.base_reset.is_none()
                    {
                        return Err("invalid unlinked graphic deletion cursor".into());
                    }
                    continue;
                }
                if d.assets_released {
                    return Err("character assets released before registry unlink".into());
                }
                if !deleting.insert(d.target)
                    || !self.unplaced.contains(&d.target)
                        && !d.cursor.as_ref().is_some_and(|e| e.may_be_onlot())
                        //A prior cursor part's42186b can place the whole group
                        //back on-lot. The validated group wrapper then calls
                        //440eeb on the next still-live part before its Reset.
                        && !(d.force_part
                            && self.vm.world.objects.get(&d.target).is_some_and(|o| o.cursor.is_some()))
                    || !self.vm.world.objects.contains_key(&d.target)
                {
                    return Err(format!(
                        "invalid recursive deletion continuation: object {}, frame {}, unplaced={}, cursor_onlot={}",
                        d.target,
                        index,
                        self.unplaced.contains(&d.target),
                        d.cursor.as_ref().is_some_and(|e| e.may_be_onlot())
                    ));
                }
                if d.slot_limit.is_none() && d.next_slot != 0
                    || d.slot_limit
                        .is_some_and(|limit| !d.notified || d.next_slot > limit)
                {
                    return Err("invalid native deletion child cursor".into());
                }
                if self.native_lot.is_some() {
                    if let Some(entry) = &d.entry {
                        entry.validate(self, p, d.target)?;
                        if d.notified != entry.complete() || self.native_selection.is_some() {
                            return Err("deletion entry ordering or ownership differs".into());
                        }
                    }
                    self.native_removal_tree(p, d.target)?;
                    if let Some(definition) = &d.entry_definition {
                        let is_cursor =
                            sims_vm::construction::definition_words(p, definition)?[9] == 9;
                        if is_cursor != d.cursor.is_some() {
                            return Err("deletion captured class dispatch differs".into());
                        }
                    }
                    if let Some(e) = &d.cursor {
                        e.validate(self, p, index)?;
                    } else if d.notified
                        && d.entry_definition.is_none()
                        && sims_vm::construction::definition_words(
                            p,
                            self.vm.world.objects[&d.target]
                                .definition
                                .as_ref()
                                .ok_or("deletion definition absent")?,
                        )?[9]
                            == 9
                    {
                        return Err("type9 deletion omitted its captured cursor owner".into());
                    }
                    if self.vm.world.objects[&d.target].person.is_none() {
                        if let Some(reset) = &d.base_reset {
                            if !d.notified || d.slot_limit.is_some() && !reset.complete() {
                                return Err("base deletion Reset ordering mismatch".into());
                            }
                            reset.validate(self, p, d.target)?;
                        } else if d.slot_limit.is_some() {
                            return Err("base deletion omitted own Reset".into());
                        }
                        if d.person_reset.is_some() {
                            return Err("person Reset on base object deletion".into());
                        }
                    } else if d.base_reset.is_some() {
                        return Err("base Reset on person deletion".into());
                    }
                    if let Some(reset) = &d.person_reset {
                        if !d.notified || d.slot_limit.is_some() && !reset.complete {
                            return Err("person deletion reset ordering mismatch".into());
                        }
                        reset.validate(self, p, d.target)?;
                    } else if self.vm.world.objects[&d.target].person.is_some()
                        && d.slot_limit.is_some()
                    {
                        return Err("person deletion omitted own reset".into());
                    }
                } else {
                    self.empty_removal_callbacks(p, d.target)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Body, Point, Room};
    use serde_json::{Value, json};
    use sims_vm::{Object, Vm, World};
    use std::collections::BTreeMap;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_detach_preserves_contained_stack_and_moves_every_child_off_lot() {
        let report: Value =
            serde_json::from_str(include_str!("../tests/fixtures/domestic-detach.json")).unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let mut s = Simulation::new(
                Vm::new(World::default(), 1),
                Room {
                    width: 256,
                    height: 256,
                    obstacles: vec![],
                },
                BTreeMap::new(),
            )
            .unwrap();
            for id in 1..=3 {
                s.vm.world.objects.insert(
                    id,
                    Object {
                        slots: vec![None; 2],
                        data: BTreeMap::from([
                            (1, 0),
                            (2, 0),
                            (3, -1),
                            (8, 0),
                            (14, -1),
                            (26, 0),
                            (29, 3),
                        ]),
                        ..Default::default()
                    },
                );
                s.bodies.insert(
                    id,
                    Body {
                        level: 1,
                        position: Point::new(72, 56),
                        elevation: 0,
                        radius: 1,
                        facing: 1,
                    },
                );
            }
            for (child, parent, key) in [(2, 1, "parent"), (3, 2, "child")] {
                if c[key].as_bool().unwrap() {
                    let slot = c[format!("{key}_slot")].as_u64().unwrap() as u16;
                    s.vm.world.objects.get_mut(&parent).unwrap().slots[slot as usize] = Some(child);
                    let child = s.vm.world.objects.get_mut(&child).unwrap();
                    child.container = Some((parent, slot));
                    child.data.extend([
                        (2, parent),
                        (3, slot as i16),
                        (14, slot as i16 + 1),
                        (26, parent),
                    ]);
                }
            }
            s.vm.world
                .objects
                .get_mut(&2)
                .unwrap()
                .data
                .insert(8, c["flags"].as_u64().unwrap() as i16);
            s.detach_tree(2).unwrap();
            for id in 1..=3 {
                let o = &s.vm.world.objects[&id];
                let pos = &s.vm.world.positions[&id];
                let actual = json!({"position":[pos.x,pos.y],"room":o.data[&29],"parent":o.data[&26],"slot":o.data[&14],"flags":o.data[&8] as u16,"slots":o.slots.iter().map(|v|v.unwrap_or(0)).collect::<Vec<_>>()});
                assert_eq!(
                    actual,
                    row["original"]["objects"][id as usize - 1],
                    "{c}, #{id}"
                );
            }
        }
    }
}
#[cfg(test)]
mod destroy_tests {
    use super::*;
    use crate::{Body, Point, Room};
    use serde_json::{Value, json};
    use sims_vm::{Definition, Module, Object, Vm, World};
    use std::collections::BTreeMap;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_recursive_destruction_deletes_children_before_parent() {
        let report: Value =
            serde_json::from_str(include_str!("../tests/fixtures/domestic-destroy.json")).unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let mut m = Module::default();
            m.object_definition_words.insert(1, vec![0; 108]);
            m.object_functions.insert(1, vec![]);
            let p = Program {
                modules: BTreeMap::from([("test".into(), m)]),
                ..Default::default()
            };
            let mut s = Simulation::new(
                Vm::new(World::default(), 1),
                Room {
                    width: 256,
                    height: 256,
                    obstacles: vec![],
                },
                BTreeMap::new(),
            )
            .unwrap();
            for id in 1..=3 {
                s.vm.world.objects.insert(
                    id,
                    Object {
                        definition: Some(Definition {
                            source: "test".into(),
                            object: 1,
                            guid: 1,
                            slots: 0,
                        }),
                        slots: vec![None; 2],
                        data: BTreeMap::from([(1, 0), (8, 0), (29, 3)]),
                        ..Default::default()
                    },
                );
                s.bodies.insert(
                    id,
                    Body {
                        level: 1,
                        position: Point::new(72, 56),
                        elevation: 0,
                        radius: 1,
                        facing: 1,
                    },
                );
            }
            if c["child"].as_bool().unwrap() {
                let slot = c["child_slot"].as_u64().unwrap() as u16;
                s.vm.world.objects.get_mut(&2).unwrap().slots[slot as usize] = Some(3);
                s.vm.world.objects.get_mut(&3).unwrap().container = Some((2, slot));
            }
            s.detach_tree(2).unwrap();
            s.removals.push(Pending {
                target: 2,
                suppress_reset: false,
            });
            let mut step = Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            assert!(s.flush_removals(&p, &mut 1000, &mut step).unwrap());
            let events = step.events;
            assert_eq!(
                json!(s.vm.world.objects.keys().collect::<Vec<_>>()),
                row["original"]["remaining"]
            );
            let expected = row["original"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[0] == "unlink")
                .map(|e| format!("deleted #{} after lot object updates", e[1]))
                .collect::<Vec<_>>();
            assert_eq!(
                events.iter().map(|e| &e.detail).collect::<Vec<_>>(),
                expected.iter().collect::<Vec<_>>(),
                "{c}"
            );
            assert!(s.removals.is_empty());
        }
    }
}
