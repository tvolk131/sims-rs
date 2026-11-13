//! Application-owned synchronous named callbacks from original 0x4764fe.
//! The UI poll owns the result; the simulation retains and validates the exact
//! existing object VM continuation across diagnostic budgets and saves.
use crate::Simulation;
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Callback {
    pub object: ObjectId,
    pub thread: ThreadId,
    pub depth: usize,
    pub owner: String,
    pub bhav: u16,
    pub query: bool,
    pub tick: u64,
}
impl Callback {
    pub fn name(&self) -> &'static str {
        if self.query {
            "query wait event"
        } else {
            "got wait event"
        }
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let vm = &s.vm;
        if self.tick != vm.tick
            || vm.object_threads.get(&self.object) != Some(&self.thread)
            || vm.resolve_named_tree(p, self.object, self.name())?
                != Some((self.owner.clone(), self.bhav))
        {
            return Err("UI wait callback origin differs".into());
        }
        let t = vm
            .threads
            .get(&self.thread)
            .ok_or("UI wait thread absent")?;
        if let Some(b) = t
            .callback
            .as_ref()
            .and_then(|b| b.chain().find(|b| b.base_depth == self.depth))
        {
            let f = t
                .stack
                .get(self.depth)
                .ok_or("UI wait callback frame absent")?;
            if !b.external
                || b.parent.is_some()
                || !t.check
                || f.caller != self.object
                || f.callee != self.object
                || f.code_owner != self.owner
                || f.location.bhav != self.bhav
            {
                return Err("UI wait callback binding differs".into());
            }
        } else if t.callback.is_some() || t.stack.len() != self.depth || t.callback_result.is_none()
        {
            return Err("UI wait callback completion absent".into());
        }
        Ok(())
    }
}
