//! Synchronous function-8 host; ordinary object scheduling remains suspended.
use super::{Split, Updates, join_neighbors};
use crate::{Simulation, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Callback {
    object: ObjectId,
    thread: ThreadId,
    depth: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updates: Option<Updates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    split: Option<Split>,
    callback: Option<Callback>,
}
impl Execution {
    pub fn begin(s: &mut Simulation, selected: ObjectId) -> Result<Self, String> {
        let links = &s
            .native_lot
            .as_ref()
            .ok_or("dynamic update lot absent")?
            .placement
            .links;
        join_neighbors(links, &mut s.vm.world, selected)?;
        let updates = Updates::begin(links, &mut s.vm.world, selected)?;
        s.publish_live_placement_caches()?;
        Ok(Self {
            updates: Some(updates),
            split: None,
            callback: None,
        })
    }
    pub fn begin_split(s: &mut Simulation, selected: ObjectId) -> Result<Self, String> {
        let split = Split::begin(&mut s.vm.world, selected)?;
        Ok(Self {
            updates: None,
            split: Some(split),
            callback: None,
        })
    }
    pub fn binding(&self) -> ObjectId {
        self.updates
            .as_ref()
            .map(Updates::binding)
            .or_else(|| self.split.as_ref().map(Split::binding))
            .unwrap_or(0)
    }
    pub fn complete(&self) -> bool {
        self.callback.is_none()
            && (self.updates.as_ref().is_some_and(Updates::complete)
                || self.split.as_ref().is_some_and(Split::complete))
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.callback.as_ref().is_some_and(|c| {
            let Some(t) = vm.threads.get(&thread) else {
                return false;
            };
            let Some(b) = t
                .callback
                .iter()
                .flat_map(|b| b.chain())
                .find(|b| b.external && b.base_depth == depth)
            else {
                return false;
            };
            let Some(f) = t.stack.get(depth) else {
                return false;
            };
            let Some(d) = vm
                .world
                .objects
                .get(&c.object)
                .and_then(|o| o.definition.as_ref())
            else {
                return false;
            };
            let action = p
                .modules
                .get(&d.source)
                .and_then(|m| m.object_functions.get(&d.object))
                .and_then(|v| v.get(8))
                .map(|f| f.action);
            c.thread == thread
                && c.depth == depth
                && b.external
                && f.caller == c.object
                && f.callee == c.object
                && f.code_owner == d.source
                && action == Some(f.location.bhav)
        })
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        let pending = match (&self.updates, &self.split) {
            (Some(updates), None) => {
                updates.validate(&vm.world)?;
                updates.pending()
            }
            (None, Some(split)) => {
                split.validate(&vm.world)?;
                split.pending()
            }
            _ => return Err("dynamic operation needs exactly one traversal".into()),
        };
        if let Some(c) = &self.callback {
            if pending != Some(c.object) || !self.owns_callback(vm, p, c.thread, c.depth) {
                return Err("invalid dynamic update callback".into());
            }
        }
        Ok(())
    }
}
impl Simulation {
    pub(crate) fn drive_dynamic_update(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if let Some(c) = e.callback.clone() {
                if self.vm.threads[&c.thread].callback.is_some() {
                    if *remaining == 0 {
                        return Ok(false);
                    }
                    let t = &self.vm.threads[&c.thread];
                    if t.status != Status::Ready {
                        return Err(format!(
                            "dynamic update callback host required: {:?}",
                            t.status
                        ));
                    }
                    let before = t.stack.clone();
                    let slice = self.vm.run_slice(p, c.thread, 1)?;
                    if slice.trace.is_empty()
                        && self.vm.threads[&c.thread].callback.is_some()
                        && self.vm.threads[&c.thread].stack == before
                        && self.vm.threads[&c.thread].status == Status::Ready
                    {
                        return Err("dynamic update callback made no progress".into());
                    }
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                    if self.vm.threads[&c.thread].callback.is_some() {
                        continue;
                    }
                }
                self.vm.threads[&c.thread]
                    .callback_result
                    .ok_or("dynamic callback result absent")?;
                e.callback = None;
                self.publish_live_placement_caches()?;
            }
            let links = &self.native_lot.as_ref().unwrap().placement.links;
            let update = match (&mut e.updates, &mut e.split) {
                (Some(updates), None) => updates.next(links, &mut self.vm.world)?,
                (None, Some(split)) => split.next(links, &mut self.vm.world)?,
                _ => return Err("dynamic operation needs exactly one traversal".into()),
            };
            let Some(update) = update else {
                return Ok(true);
            };
            let depth = self
                .vm
                .object_threads
                .get(&update.object)
                .and_then(|t| self.vm.threads.get(t))
                .map_or(0, |t| t.stack.len());
            if let Some(thread) = self.vm.begin_object_function_with_args(
                p,
                update.object,
                8,
                0,
                Some(&update.args),
            )? {
                e.callback = Some(Callback {
                    object: update.object,
                    thread,
                    depth,
                });
            }
        }
    }
}
