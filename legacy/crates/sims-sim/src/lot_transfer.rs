//! Original Grab, Drop and Drop Onto wrappers with shared native acceptance
//! and SetObject. The primitive resumes only after synchronous callbacks finish.
use crate::{Simulation, Step, lot_acceptance, lot_links::Descriptor, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm, World, transfer::Request};

/// Arguments passed to the object's virtual placement check. In Drop Onto,
/// the public checker replaces (-16,-16) with the destination's current X/Y,
/// but retains the literal level 1 until SetObject commits.
pub fn destination(world: &World, r: &Request) -> Result<Option<(ObjectId, Descriptor)>, String> {
    if let Some([source, slot]) = r.onto {
        let Some(id) = sims_vm::transfer::onto_object(world, r.caller, r.target, source)? else {
            return Ok(None);
        };
        if !world.objects.contains_key(&id) {
            return Err("Drop Onto has an unregistered raw slot occupant".into());
        }
        return Ok(Some((
            id,
            Descriptor {
                on_lot: true,
                position: [-16, -16],
                container: Some(r.target),
                slot: i32::from(slot),
                level: 1,
            },
        )));
    }
    let Some(id) = sims_vm::transfer::object(world, r.caller, r.target, r.grab) else {
        return Ok(None);
    };
    let at = world
        .positions
        .get(&r.caller)
        .ok_or("native transfer caller position absent")?;
    let position = if r.grab {
        [at.x, at.y]
    } else {
        let direction = *world.objects[&r.caller]
            .data
            .get(&1)
            .ok_or("native drop direction absent")?;
        if !(0..8).contains(&direction) {
            return Err("native drop direction outside offset table".into());
        }
        sims_vm::create::in_front([at.x, at.y], direction as u8)
    };
    Ok(Some((
        id,
        Descriptor {
            on_lot: true,
            position,
            container: r.grab.then_some(r.caller),
            slot: 0,
            level: at.level,
        },
    )))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    thread: ThreadId,
    request: Request,
    object: ObjectId,
    destination: Descriptor,
    acceptance: lot_acceptance::live::Execution,
    placement: Option<lot_placement::live::Execution>,
}
impl Execution {
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.acceptance.owns_callback(vm, p, thread, depth)
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program, key: u64) -> Result<(), String> {
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("native transfer thread absent")?;
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
            .any(|status| matches!(status,Status::AwaitingTransfer(r) if r==&self.request));
        let expected_container = if self.request.onto.is_some() {
            Some(self.request.target)
        } else {
            self.request.grab.then_some(self.request.caller)
        };
        let expected_slot = self.request.onto.map_or(0, |a| i32::from(a[1]));
        if s.native_lot.is_none()
            || !bound
            || key != self.request.id
            || self.destination.container != expected_container
            || self.destination.slot != expected_slot
            || !self.destination.on_lot
            || self.request.onto.is_some() && self.destination.level != 1
            || self.request.grab && self.object != self.request.target
        {
            return Err("unbound native transfer continuation".into());
        }
        self.acceptance.validate(&s.vm, p)?;
        if self.acceptance.binding().0 != self.object
            || self.acceptance.binding().2 != self.destination.level
            || self.acceptance.container_binding() != expected_container.map(|c| (c, expected_slot))
            || expected_container.is_none()
                && self.acceptance.binding().1 != self.destination.position
        {
            return Err("native transfer acceptance mismatch".into());
        }
        if let Some(e) = &self.placement {
            if !self.acceptance.outcome().is_some_and(|o| o.accepted)
                || e.operation.binding() != (self.object, Some(self.destination))
            {
                return Err("native transfer placement mismatch".into());
            }
            e.validate(&s.vm, p, self.object)?;
        }
        Ok(())
    }
}
impl Simulation {
    pub(crate) fn drive_pending_native_transfers(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        while let Some((&id, _)) = self.native_transfers.last_key_value() {
            if !self.drive_native_transfer(p, id, remaining, step)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn service_native_transfer(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: &Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if !self.native_transfers.contains_key(&r.id) {
            let Some((object, to)) = destination(&self.vm.world, r)? else {
                self.vm.finish_transfer(p, thread, r.id, false)?;
                return Ok(true);
            };
            let o = &self.vm.world.objects[&object];
            let d = o
                .definition
                .as_ref()
                .ok_or("native transfer resource absent")?;
            let words = p
                .modules
                .get(&d.source)
                .and_then(|m| m.object_definition_words.get(&d.object))
                .filter(|w| w.len() > 10)
                .ok_or("native transfer OBJD absent")?;
            if o.person.is_some() || words[10] != 0 || words[9] == 9 {
                return Err("native transfer subclass placement adapter required".into());
            }
            let acceptance = if let Some(parent) = to.container {
                lot_acceptance::live::Execution::new_contained(
                    self, object, parent, to.slot, to.level,
                )?
            } else {
                lot_acceptance::live::Execution::new(self, object, to.position, to.level)?
            };
            self.native_transfers.insert(
                r.id,
                Execution {
                    thread,
                    request: r.clone(),
                    object,
                    destination: to,
                    acceptance,
                    placement: None,
                },
            );
        }
        self.drive_native_transfer(p, r.id, remaining, step)
    }
    fn drive_native_transfer(
        &mut self,
        p: &Program,
        key: u64,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let mut e = self
            .native_transfers
            .remove(&key)
            .ok_or("native transfer continuation absent")?;
        let result = self.drive_native_transfer_inner(p, &mut e, remaining, step);
        if !matches!(result, Ok(true)) {
            self.native_transfers.insert(key, e);
        }
        result
    }
    fn drive_native_transfer_inner(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if e.placement.is_none() {
            if !self.drive_native_acceptance(p, &mut e.acceptance, remaining, step)? {
                return Ok(false);
            }
            if !e.acceptance.outcome().unwrap().accepted {
                self.vm.finish_transfer(p, e.thread, e.request.id, false)?;
                return Ok(true);
            }
            e.placement = Some(lot_placement::live::Execution::new(
                lot_placement::Operation::set(e.object, e.destination, true),
            ));
        }
        if !self.drive_live_placement(p, e.placement.as_mut().unwrap(), remaining, step)? {
            return Ok(false);
        }
        self.sync_carry(p)?;
        self.vm.finish_transfer(p, e.thread, e.request.id, true)?;
        self.event(
            &mut step.events,
            e.thread,
            format!(
                "native transfer #{} into {:?} slot {} at {:?}/{}",
                e.object,
                e.destination.container,
                e.destination.slot,
                e.destination.position,
                e.destination.level
            ),
        );
        Ok(true)
    }
}
