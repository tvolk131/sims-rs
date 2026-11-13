//! Base virtual reset during ordered, frozen lot loading (0x431fbe, argument 0).
//! Person reset and the enclosing other-person notifications are separate.
pub mod live;
use crate::{
    architecture::Architecture,
    lot_links::Descriptor,
    lot_placement::{self, State, VirtualOperation},
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm, construction::ObjectDate};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Placement(lot_placement::Request),
    /// Complete this VM host primitive before advancing again. Lot time stays
    /// frozen; Init callbacks keep their native synchronous restrictions.
    Host {
        thread: ThreadId,
        status: Status,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reset {
    object: ObjectId,
    phase: u8,
    saved: Option<Descriptor>,
    placement: Option<VirtualOperation>,
    callback: Option<sims_vm::loading::BaseCallbackReset>,
    parts: Vec<ObjectId>,
    part: usize,
    updating: bool,
}
impl Reset {
    pub fn new(object: ObjectId) -> Self {
        Self {
            object,
            phase: 0,
            saved: None,
            placement: None,
            callback: None,
            parts: vec![],
            part: 0,
            updating: false,
        }
    }
    /// `marked_for_deletion` is the selected object's native +0x114 bit 0x40,
    /// observed after virtual removal. The host must supply its current value.
    pub fn advance(
        &mut self,
        vm: &mut Vm,
        p: &Program,
        a: &Architecture,
        state: &mut State,
        now: ObjectDate,
        marked_for_deletion: bool,
    ) -> Result<Option<Request>, String> {
        loop {
            match self.phase {
                0 => {
                    let o = vm
                        .world
                        .objects
                        .get(&self.object)
                        .ok_or("reset object absent")?;
                    let d = o.definition.as_ref().ok_or("reset definition absent")?;
                    let m = &p.modules[&d.source];
                    if o.person.is_some() || m.object_definition_words[&d.object][9] == 2 {
                        return Err("loaded person reset needs its own virtual ordering".into());
                    }
                    if m.object_functions[&d.object]
                        .get(3)
                        .is_some_and(|f| f.action != 0)
                    {
                        self.callback = Some(vm.begin_loaded_base_callback_reset(p, self.object)?);
                        self.phase = 1;
                    } else {
                        self.saved = Some(state.links.capture(&mut vm.world, self.object)?);
                        self.placement = Some(VirtualOperation::remove(
                            &vm.world,
                            &state.links,
                            self.object,
                            true,
                        )?);
                        self.phase = 2;
                    }
                }
                1 => {
                    let thread = self.callback.as_ref().unwrap().thread();
                    if let Some(request) = run(vm, p, thread, true)? {
                        return Ok(Some(request));
                    }
                    self.callback.take().unwrap().finish(vm, p)?;
                    self.phase = 6;
                }
                2 => {
                    if let Some(request) = self
                        .placement
                        .as_mut()
                        .unwrap()
                        .advance_loaded(vm, p, a, state)?
                    {
                        return Ok(Some(Request::Placement(request)));
                    }
                    self.placement = None;
                    if marked_for_deletion {
                        self.phase = 5;
                        continue;
                    }
                    self.parts = if vm.world.objects[&self.object].multitile.is_some() {
                        sims_vm::multitile::chain(&vm.world, self.object)?
                    } else {
                        vec![self.object]
                    };
                    // cXMTObject resets all storage before any Init/Update.
                    for &id in &self.parts {
                        vm.reset_loaded_storage(p, id, now)?;
                    }
                    if vm.world.objects[&self.object].multitile.is_some() {
                        sims_vm::multitile::update_offsets(&mut vm.world, self.object)?;
                    }
                    self.phase = 3;
                }
                3 => {
                    if self.part == self.parts.len() {
                        let saved = self.saved.unwrap();
                        if saved.on_lot {
                            self.placement = Some(VirtualOperation::set(
                                &vm.world,
                                &state.links,
                                self.object,
                                saved,
                                true,
                            )?);
                            self.phase = 4;
                        } else {
                            self.phase = 5;
                        }
                        continue;
                    }
                    let id = self.parts[self.part];
                    if !self.updating {
                        if vm.world.objects[&id].multitile.is_some() {
                            vm.begin_multitile_init(p, id)?;
                        }
                        self.updating = true;
                    }
                    let thread = vm.object_threads[&id];
                    if let Some(request) = run(vm, p, thread, false)? {
                        return Ok(Some(request));
                    }
                    vm.finish_object_update(id, true)?;
                    vm.refresh_registry_flags(p, id)?;
                    self.updating = false;
                    self.part += 1;
                }
                4 => {
                    if let Some(request) = self
                        .placement
                        .as_mut()
                        .unwrap()
                        .advance_loaded(vm, p, a, state)?
                    {
                        return Ok(Some(Request::Placement(request)));
                    }
                    self.placement = None;
                    self.phase = 5;
                }
                5 => {
                    vm.finish_loaded_reset(self.object)?;
                    self.phase = 6;
                }
                _ => return Ok(None),
            }
        }
    }
}
fn run(
    vm: &mut Vm,
    p: &Program,
    thread: ThreadId,
    callback: bool,
) -> Result<Option<Request>, String> {
    let mut remaining = 100_000;
    loop {
        if remaining == 0 {
            return Err("loaded reset exceeded diagnostic instruction budget".into());
        }
        let slice = vm.run_slice(p, thread, remaining)?;
        remaining -= slice.trace.len();
        let t = &vm.threads[&thread];
        match &t.status {
            Status::Ready | Status::Sleeping { .. } | Status::Returned(_)
                if callback && t.callback.is_none() =>
            {
                return Ok(None);
            }
            Status::Ready if !slice.trace.is_empty() => {}
            Status::Sleeping { .. } | Status::Returned(_) if !callback && t.callback.is_none() => {
                return Ok(None);
            }
            Status::Faulted(fault) => return Err(format!("loaded reset: {fault:?}")),
            status => {
                return Ok(Some(Request::Host {
                    thread,
                    status: status.clone(),
                }));
            }
        }
    }
}
