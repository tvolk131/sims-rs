//! Budgeted base virtual Reset(0), 0x431fbe. Its owner freezes ordinary lot
//! scheduling and retains this controller until completion. Person notifications
//! and person Reset are separate; this never substitutes a storage reset for them.
use crate::{Simulation, Step, lot_links::Descriptor, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm, construction::ObjectDate};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Begin,
    Callback(sims_vm::loading::BaseCallbackReset),
    Detach {
        saved: Descriptor,
        placement: lot_placement::live::Execution,
    },
    Update {
        saved: Descriptor,
        parts: Vec<ObjectId>,
        cursor: usize,
        started: bool,
    },
    Reinsert(lot_placement::live::Execution),
    Finish,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    object: ObjectId,
    phase: Phase,
}
impl Execution {
    pub fn new(object: ObjectId) -> Self {
        Self {
            object,
            phase: Phase::Begin,
        }
    }
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        match &self.phase {
            Phase::Callback(e) => e.owns_callback(vm, p, thread, depth),
            Phase::Detach { placement, .. } | Phase::Reinsert(placement) => {
                placement.owns_callback(vm, p, thread, depth)
            }
            _ => false,
        }
    }
    fn has_tree(&self, s: &Simulation, p: &Program) -> Result<bool, String> {
        let object =
            s.vm.world
                .objects
                .get(&self.object)
                .ok_or("base Reset object absent")?;
        let d = object
            .definition
            .as_ref()
            .ok_or("base Reset definition absent")?;
        let m = p.modules.get(&d.source).ok_or("base Reset module absent")?;
        let kind = m
            .object_definition_words
            .get(&d.object)
            .and_then(|w| w.get(9))
            .ok_or("base Reset object kind absent")?;
        if object.person.is_some() || *kind == 2 {
            return Err("person Reset requires its own virtual ordering".into());
        }
        Ok(m.object_functions
            .get(&d.object)
            .ok_or("base Reset functions absent")?
            .get(3)
            .is_some_and(|f| f.action != 0))
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let has_tree = self.has_tree(s, p)?;
        if s.native_lot.is_none() || !s.vm.object_threads.contains_key(&self.object) {
            return Err("base Reset requires a live native object".into());
        }
        s.validate_native_lot()?;
        match &self.phase {
            Phase::Callback(e) => {
                if !has_tree || e.object() != self.object {
                    return Err("base Reset callback dispatch changed".into());
                }
                e.validate(&s.vm, p)?;
            }
            Phase::Detach { placement, .. } => {
                if has_tree || placement.operation.binding() != (self.object, None) {
                    return Err("base Reset detach binding changed".into());
                }
                placement.validate(&s.vm, p, self.object)?;
            }
            Phase::Reinsert(placement) => {
                if has_tree
                    || !matches!(placement.operation.binding(), (id, Some(d)) if id == self.object && d.on_lot)
                {
                    return Err("base Reset reinsert binding changed".into());
                }
                placement.validate(&s.vm, p, self.object)?;
            }
            Phase::Update {
                parts,
                cursor,
                started,
                ..
            } => {
                let expected = if s.vm.world.objects[&self.object].multitile.is_some() {
                    sims_vm::multitile::chain(&s.vm.world, self.object)?
                } else {
                    vec![self.object]
                };
                if has_tree
                    || *parts != expected
                    || *cursor > parts.len()
                    || *cursor == parts.len() && *started
                    || parts.iter().any(|id| !s.vm.lifecycles.contains_key(id))
                    || parts
                        .iter()
                        .take(*cursor)
                        .any(|id| s.vm.world.objects[id].last_update != Some(s.vm.tick))
                    || parts
                        .iter()
                        .skip(*cursor + usize::from(*started))
                        .any(|id| s.vm.world.objects[id].last_update.is_some())
                {
                    return Err("base Reset storage/update binding changed".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// The supplied flag is native object +0x114 bit 0x40, not registry flags.
    /// Pending live removals are also read after detach callbacks. A budget
    /// yield preserves this operation, its callback frames and its saved location.
    pub fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        now: ObjectDate,
        marked_for_deletion: bool,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            match &mut self.phase {
                Phase::Begin => {
                    if self.has_tree(s, p)? {
                        self.phase =
                            Phase::Callback(s.vm.begin_loaded_base_callback_reset(p, self.object)?);
                    } else {
                        let native = s
                            .native_lot
                            .as_ref()
                            .ok_or("base Reset native lot absent")?;
                        let saved = native
                            .placement
                            .links
                            .capture(&mut s.vm.world, self.object)?;
                        let operation = lot_placement::VirtualOperation::remove(
                            &s.vm.world,
                            &native.placement.links,
                            self.object,
                            true,
                        )?;
                        self.phase = Phase::Detach {
                            saved,
                            placement: lot_placement::live::Execution::new_virtual(operation),
                        };
                    }
                }
                Phase::Callback(e) => {
                    let thread = e.thread();
                    while s.vm.threads[&thread].callback.is_some() {
                        if *remaining == 0 {
                            return Ok(false);
                        }
                        if s.vm.threads[&thread].status != Status::Ready {
                            return Err(format!(
                                "base Reset callback host required: {:?}",
                                s.vm.threads[&thread].status
                            ));
                        }
                        let slice = s.vm.run_slice(p, thread, 1)?;
                        *remaining -= slice.trace.len();
                        step.trace.extend(slice.trace);
                    }
                    e.clone().finish(&mut s.vm, p)?;
                    self.phase = Phase::Complete;
                }
                Phase::Detach { saved, placement } => {
                    if !s.drive_live_placement(p, placement, remaining, step)? {
                        return Ok(false);
                    }
                    if marked_for_deletion
                        || s.removals.iter().any(|r| r.target == self.object)
                        || s.deletion.iter().any(|d| d.target == self.object)
                    {
                        self.phase = Phase::Finish;
                        continue;
                    }
                    let parts = if s.vm.world.objects[&self.object].multitile.is_some() {
                        sims_vm::multitile::chain(&s.vm.world, self.object)?
                    } else {
                        vec![self.object]
                    };
                    // Native cXMTObject resets every member's storage before
                    // running any member's Init/immediate update.
                    for &id in &parts {
                        s.vm.reset_loaded_storage(p, id, now)?;
                    }
                    if s.vm.world.objects[&self.object].multitile.is_some() {
                        sims_vm::multitile::update_offsets(&mut s.vm.world, self.object)?;
                    }
                    self.phase = Phase::Update {
                        saved: *saved,
                        parts,
                        cursor: 0,
                        started: false,
                    };
                }
                Phase::Update {
                    saved,
                    parts,
                    cursor,
                    started,
                } => {
                    if *cursor == parts.len() {
                        self.phase = if saved.on_lot {
                            Phase::Reinsert(lot_placement::live::Execution::new_virtual(
                                lot_placement::VirtualOperation::set(
                                    &s.vm.world,
                                    &s.native_lot.as_ref().unwrap().placement.links,
                                    self.object,
                                    *saved,
                                    true,
                                )?,
                            ))
                        } else {
                            Phase::Finish
                        };
                        continue;
                    }
                    let id = parts[*cursor];
                    if !*started {
                        if s.vm.world.objects[&id].multitile.is_some() {
                            s.vm.begin_multitile_init(p, id)?;
                        }
                        *started = true;
                    }
                    // Use the immediate-update driver, not finish_object_factory:
                    // reconstruction must not apply a new catalog-price store.
                    if !s.drive_factory_update(p, id, remaining, step)? {
                        return Ok(false);
                    }
                    s.vm.finish_object_update(id, true)?;
                    s.vm.refresh_registry_flags(p, id)?;
                    *started = false;
                    *cursor += 1;
                }
                Phase::Reinsert(placement) => {
                    if !s.drive_live_placement(p, placement, remaining, step)? {
                        return Ok(false);
                    }
                    self.phase = Phase::Finish;
                }
                Phase::Finish => {
                    s.vm.finish_loaded_reset(self.object)?;
                    self.phase = Phase::Complete;
                }
                Phase::Complete => return Ok(true),
            }
        }
    }
}
