//!Live services for42231d/423175. Explicit Update ownership permits both
//!source calls at the same game time; instruction budgets are not new ticks.
use super::id;
use crate::{Simulation, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{
    Program, Status, Stop, ThreadId, Vm,
    construction::ObjectDate,
    cursor::reset::{Borrowed, Call, Request},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Child {
    BaseInit {
        object: u32,
        thread: ThreadId,
        update: i16,
        run: bool,
    },
    Init {
        object: u32,
        thread: ThreadId,
        depth: usize,
    },
    Update {
        object: u32,
        thread: ThreadId,
        base: bool,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Execution {
    call: Call,
    child: Option<Child>,
}
impl Execution {
    pub(super) fn new(object: u32, update: i16) -> Result<Self, String> {
        Ok(Self {
            call: Call::new(object, update)?,
            child: None,
        })
    }
    pub(super) fn complete(&self) -> bool {
        self.call.complete() && self.child.is_none()
    }
    pub(super) fn binding(&self) -> (u32, i16) {
        self.call.binding()
    }
    pub(super) fn owns_visit(&self, vm: &Vm) -> bool {
        let Some(Child::Update { object, thread, .. }) = self.child else {
            return false;
        };
        vm.scheduled_object_visit.as_ref().is_some_and(|v| {
            v.chain().count() == 1
                && Some(v.object()) == id(vm, object).ok()
                && v.thread() == thread
        })
    }
    pub(super) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        let Some(Child::Init {
            object,
            thread: saved,
            depth: base,
        }) = self.child
        else {
            return false;
        };
        if thread != saved || depth != base {
            return false;
        }
        let Ok(id) = id(vm, object) else {
            return false;
        };
        let Some(d) = vm.world.objects[&id].definition.as_ref() else {
            return false;
        };
        let action = p
            .modules
            .get(&d.source)
            .and_then(|m| m.object_functions.get(&d.object))
            .and_then(|f| f.first())
            .map(|f| f.action);
        vm.threads.get(&thread).is_some_and(|t| {
            t.callback
                .iter()
                .flat_map(|c| c.chain())
                .any(|c| c.external && c.base_depth == depth)
                && t.stack.get(depth).is_some_and(|f| {
                    f.caller == id
                        && f.callee == id
                        && f.code_owner == d.source
                        && Some(f.location.bhav) == action
                })
        })
    }
    pub(super) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        self.call.validate()?;
        if self.complete() && self.call.pending().is_some() {
            return Err("completed cursor Reset has pending service".into());
        }
        if let Some(c) = &self.child {
            let (h, thread) = match *c {
                Child::BaseInit {
                    object,
                    thread,
                    update,
                    run,
                } => {
                    if self.call.pending() != Some(Request::BaseReset { object, update }) {
                        return Err("cursor base Reset binding differs".into());
                    }
                    if s.vm
                        .lifecycles
                        .get(&id(&s.vm, object)?)
                        .is_none_or(|l| l.run_init != run || l.thread != thread)
                    {
                        return Err("cursor base Reset lifecycle differs".into());
                    }
                    (object, thread)
                }
                Child::Init {
                    object,
                    thread,
                    depth,
                } => {
                    if self.call.pending() != Some(Request::Init(object))
                        || (s
                            .vm
                            .threads
                            .get(&thread)
                            .is_some_and(|t| t.callback.is_some())
                            && !self.owns_callback(&s.vm, p, thread, depth))
                    {
                        return Err("cursor Init binding differs".into());
                    }
                    (object, thread)
                }
                Child::Update {
                    object,
                    thread,
                    base,
                } => {
                    let bound = if base {
                        matches!(self.call.pending(), Some(Request::BaseReset { object: h, update }) if h == object && update != 0)
                    } else {
                        self.call.pending() == Some(Request::Update(object))
                    };
                    if !bound || !self.owns_visit(&s.vm) {
                        return Err("cursor Update binding differs".into());
                    }
                    (object, thread)
                }
            };
            let object = id(&s.vm, h)?;
            if s.vm.object_threads.get(&object) != Some(&thread)
                || s.vm.world.objects[&object]
                    .cursor
                    .as_ref()
                    .is_none_or(|c| !c.postconstructed)
            {
                return Err("cursor Reset child lost its registered lifecycle".into());
            }
        }
        Ok(())
    }
    fn start_update(&mut self, s: &mut Simulation, object: u32, base: bool) -> Result<(), String> {
        let id = id(&s.vm, object)?;
        s.vm.begin_scheduled_object_visit(id)?;
        self.child = Some(Child::Update {
            object,
            thread: s.vm.object_threads[&id],
            base,
        });
        Ok(())
    }
    pub(super) fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        date: ObjectDate,
        budget: &mut usize,
        step: &mut Step,
    ) -> Result<(), String> {
        while *budget > 0 && !self.complete() {
            if let Some(child) = self.child.clone() {
                let thread = match child {
                    Child::BaseInit { thread, .. }
                    | Child::Init { thread, .. }
                    | Child::Update { thread, .. } => thread,
                };
                if matches!(child, Child::Update { .. }) {
                    let r = s.vm.run_slice(p, thread, 1)?;
                    *budget -= r.trace.len().max(1);
                    step.trace.extend(r.trace);
                    match r.stop {
                        Stop::Budget => continue,
                        Stop::Sleeping | Stop::Returned(_) => (),
                        other => {
                            return Err(format!("cursor Update requires live host: {other:?}"));
                        }
                    }
                    let Child::Update { object, base, .. } = child else {
                        unreachable!()
                    };
                    let id = id(&s.vm, object)?;
                    s.vm.finish_object_update(id, true)?;
                    s.vm.end_scheduled_object_visit()?;
                    if base {
                        s.vm.refresh_registry_flags(p, id)?;
                    }
                    self.call
                        .acknowledge(self.call.pending().ok_or("cursor Update request absent")?)?;
                    self.child = None;
                    continue;
                }
                let t =
                    s.vm.threads
                        .get(&thread)
                        .ok_or("cursor Init thread absent")?;
                if t.callback.is_some() {
                    if t.status != Status::Ready {
                        return Err(format!("cursor Init requires live host: {:?}", t.status));
                    }
                    let r = s.vm.run_slice(p, thread, 1)?;
                    *budget -= r.trace.len().max(1);
                    step.trace.extend(r.trace);
                    if s.vm.threads[&thread].callback.is_some() {
                        continue;
                    }
                }
                match child {
                    Child::BaseInit {
                        object,
                        update,
                        run,
                        ..
                    } => {
                        if run && update != 0 {
                            self.start_update(s, object, true)?;
                            continue;
                        }
                        if run {
                            s.vm.refresh_registry_flags(p, id(&s.vm, object)?)?;
                        }
                    }
                    Child::Init { .. } => {
                        s.vm.threads[&thread]
                            .callback_result
                            .ok_or("cursor Init result absent")?;
                    }
                    Child::Update { .. } => unreachable!(),
                }
                self.call
                    .acknowledge(self.call.pending().ok_or("cursor Init request absent")?)?;
                self.child = None;
                continue;
            }
            let Some(request) = self.call.advance(&mut Borrowed {
                world: &mut s.vm.world,
            })?
            else {
                break;
            };
            *budget -= 1;
            match request {
                Request::BaseReset { object, update } => {
                    let id = id(&s.vm, object)?;
                    let thread = s.vm.begin_cursor_base_reset(p, id, date)?;
                    self.child = Some(Child::BaseInit {
                        object,
                        thread,
                        update,
                        run: s.vm.lifecycles[&id].run_init,
                    });
                    continue;
                }
                Request::Init(object) => {
                    let id = id(&s.vm, object)?;
                    let depth =
                        s.vm.object_threads
                            .get(&id)
                            .and_then(|t| s.vm.threads.get(t))
                            .map_or(0, |t| t.stack.len());
                    if let Some(thread) = s.vm.begin_loaded_function(p, id, 0)? {
                        self.child = Some(Child::Init {
                            object,
                            thread,
                            depth,
                        });
                        continue;
                    }
                }
                Request::Update(object) => {
                    self.start_update(s, object, false)?;
                    continue;
                }
                Request::Offsets(object) => {
                    let id = id(&s.vm, object)?;
                    sims_vm::cursor::refresh_offsets(&mut s.vm.world, p, id)?;
                }
                Request::RegistryFlags(object) => {
                    s.vm.refresh_registry_flags(p, id(&s.vm, object)?)?
                }
                Request::Invalidate(object) => {
                    let id = id(&s.vm, object)?;
                    s.vm.refresh_object(id, id, 0, 0)?;
                }
                Request::Flags { object, low } => {
                    s.update_native_low_flags(p, id(&s.vm, object)?, low)?
                }
            }
            self.call.acknowledge(request)?;
        }
        Ok(())
    }
}
