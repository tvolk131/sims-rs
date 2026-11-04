//! Cross-object non-yielding named trees (0x44ba26 mode 1 -> 0x4749b5).
//! The recipient keeps its existing frame arena and ordinary continuation.
//! This stack represents synchronous native calls across VMs; diagnostic
//! instruction budgets may suspend them, but the lot scheduler may not run.
use crate::{Program, Run, Start, Status, Stop, Thread, ThreadId, Vm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    pub requester: ThreadId,
    pub depth: usize,
    pub recipient: ThreadId,
    pub object: crate::ObjectId,
    pub owner: String,
    pub bhav: u16,
    pub base_depth: usize,
    pub result: Option<bool>,
}

impl Vm {
    pub(crate) fn local_named_callback(&self, p: &Program, t: &Thread, depth: usize) -> bool {
        let Some(parent) = depth.checked_sub(1).and_then(|i| t.stack.get(i)) else {
            return false;
        };
        let Ok((_, routine)) = p.resolve(&parent.code_owner, parent.location.bhav) else {
            return false;
        };
        let Some(ins) = routine.body.instructions.get(parent.location.instruction) else {
            return false;
        };
        let Some(child) = t.stack.get(depth) else {
            return false;
        };
        ins.opcode == 28
            && parent.continuation == 2
            && (ins.operands[5] == 0
                || ins.operands[5] == 1 && parent.stack_object == parent.caller)
            && self
                .named_tree_binding(p, parent, &ins.operands)
                .ok()
                .flatten()
                == Some((child.code_owner.clone(), child.location.bhav))
            && child.caller == parent.caller
            && child.callee == parent.stack_object
    }
    pub(crate) fn push_named_callback(
        &mut self,
        p: &Program,
        id: ThreadId,
        t: &mut Thread,
        owner: String,
        bhav: u16,
        object: crate::ObjectId,
    ) -> Result<(), String> {
        let recipient = *self
            .object_threads
            .get(&object)
            .ok_or("named callback recipient VM absent")?;
        if recipient == id || self.named_calls.len() >= crate::MAX_STACK {
            return Err("invalid cross-object named callback entry".into());
        }
        let target = self
            .threads
            .get(&recipient)
            .ok_or("named callback recipient thread absent")?;
        if matches!(target.status, Status::Faulted(_)) {
            return Err(
                "named callback on faulted recipient needs native error-state adapter".into(),
            );
        }
        let base_depth = target.stack.len();
        self.begin_immediate_callback(
            p,
            recipient,
            Start {
                owner: &owner,
                bhav,
                caller: object,
                callee: object,
                stack_object: object,
                args: self.world.objects[&object].temps.to_vec(),
                check: true,
            },
        )?;
        t.stack.last_mut().unwrap().continuation = 3;
        self.named_calls.push(Call {
            requester: id,
            depth: t.stack.len(),
            recipient,
            object,
            owner,
            bhav,
            base_depth,
            result: None,
        });
        Ok(())
    }

    pub fn owns_named_callback(&self, thread: ThreadId, depth: usize) -> bool {
        self.named_calls
            .iter()
            .any(|c| c.recipient == thread && c.base_depth == depth && c.result.is_none())
    }

    /// Execute one logical caller, including synchronous calls on other VMs.
    /// Every original instruction is charged to the same diagnostic budget.
    pub fn run_slice(&mut self, p: &Program, id: ThreadId, budget: usize) -> Result<Run, String> {
        // Reject before accessory publication or selecting another named caller.
        self.reject_native_suspension_run(id)?;
        if self.scheduled_object_visit.as_ref().is_some_and(|visit| {
            visit.thread() == id
                && visit.finished()
                && self.threads.get(&id).is_some_and(|thread| {
                    !thread.check && thread.callback.is_none() && thread.service.is_none()
                })
        }) {
            return Err("native object update has already returned".into());
        }
        if self.scheduled_object_visit.as_ref().is_some_and(|visit| {
            visit.thread() != id
                && self.threads.get(&id).is_none_or(|thread| {
                    !thread.check && thread.callback.is_none() && thread.service.is_none()
                })
        }) {
            return Err("native object visit owns ordinary execution".into());
        }
        if self
            .player_menu
            .as_ref()
            .is_some_and(|q| !q.allows_run(self, id))
        {
            return Err("player-menu query owns synchronous execution".into());
        }
        self.resume_accessory_assets(p);
        if self.named_calls.first().is_some_and(|c| c.requester != id) {
            return Err("another VM owns synchronous named execution".into());
        }
        let mut trace = vec![];
        let mut remaining = budget;
        loop {
            let active = self.named_calls.last().map_or(id, |c| {
                if c.result.is_some() {
                    c.requester
                } else {
                    c.recipient
                }
            });
            let run = self.run_local_slice(p, active, remaining)?;
            remaining = remaining
                .checked_sub(run.trace.len())
                .ok_or("named callback exceeded instruction budget")?;
            trace.extend(run.trace);
            if matches!(run.stop, Stop::AwaitingAccessory | Stop::AwaitingUiEffect) {
                return Ok(Run {
                    stop: run.stop,
                    trace,
                });
            }
            if let Some(c) = self.named_calls.last_mut() {
                if c.result.is_none() {
                    let target = self
                        .threads
                        .get_mut(&c.recipient)
                        .ok_or("named callback recipient disappeared")?;
                    if !target
                        .callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .any(|b| b.base_depth == c.base_depth)
                    {
                        c.result = Some(
                            target
                                .callback_result
                                .take()
                                .ok_or("named callback result absent")?,
                        );
                    } else if matches!(target.status, Status::Faulted(_)) {
                        let error =
                            format!("cross-object named callback faulted: {:?}", target.status);
                        crate::fault(self.threads.get_mut(&id).unwrap(), error);
                        return Ok(Run {
                            stop: Stop::Faulted,
                            trace,
                        });
                    } else if target.status != Status::Ready {
                        return Err(
                            "non-yielding named callback attempted to yield (native error 0x44f)"
                                .into(),
                        );
                    }
                }
            } else {
                return Ok(Run {
                    stop: run.stop,
                    trace,
                });
            }
            if remaining == 0 || run.stop == Stop::ActionCallbacks {
                return Ok(Run {
                    stop: if run.stop == Stop::ActionCallbacks {
                        run.stop
                    } else {
                        Stop::Budget
                    },
                    trace,
                });
            }
        }
    }

    pub(crate) fn validate_named_calls(&self, p: &Program) -> Result<(), String> {
        if self.named_calls.len() > crate::MAX_STACK {
            return Err("oversized named callback stack".into());
        }
        let mut sources = std::collections::BTreeSet::new();
        let mut targets = std::collections::BTreeSet::new();
        for (index, c) in self.named_calls.iter().enumerate() {
            let source = self
                .threads
                .get(&c.requester)
                .ok_or("named callback requester absent")?;
            let target = self
                .threads
                .get(&c.recipient)
                .ok_or("named callback recipient absent")?;
            let parent = c
                .depth
                .checked_sub(1)
                .and_then(|i| source.stack.get(i))
                .ok_or("named callback parent absent")?;
            let (_, routine) = p.resolve(&parent.code_owner, parent.location.bhav)?;
            let ins = routine
                .body
                .instructions
                .get(parent.location.instruction)
                .ok_or("named callback parent PC")?;
            if c.requester == c.recipient
                || parent.continuation != 3
                || ins.opcode != 28
                || ins.operands[5] != 1
                || !sources.insert((c.requester, c.depth))
                || !targets.insert((c.recipient, c.base_depth))
                || self.object_threads.get(&c.object) != Some(&c.recipient)
                || parent.stack_object != c.object
                || self.named_tree_binding(p, parent, &ins.operands)?
                    != Some((c.owner.clone(), c.bhav))
                || index > 0 && self.named_calls[index - 1].recipient != c.requester
                || c.result.is_some() && index + 1 != self.named_calls.len()
            {
                return Err("invalid named callback binding".into());
            }
            if c.result.is_none() {
                let boundary = target
                    .callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .find(|b| b.base_depth == c.base_depth)
                    .ok_or("named callback boundary absent")?;
                let child = target
                    .stack
                    .get(c.base_depth)
                    .ok_or("named callback frame absent")?;
                if !boundary.external
                    || child.caller != c.object
                    || child.callee != c.object
                    || child.code_owner != c.owner
                    || child.location.bhav != c.bhav
                {
                    return Err("invalid named callback recipient frame".into());
                }
            } else if target.stack.len() != c.base_depth {
                return Err("completed named callback retains child frames".into());
            }
        }
        for (&id, t) in &self.threads {
            for (depth, f) in t.stack.iter().enumerate() {
                if f.continuation == 3 {
                    let (_, r) = p.resolve(&f.code_owner, f.location.bhav)?;
                    if r.body.instructions[f.location.instruction].opcode == 28
                        && !sources.contains(&(id, depth + 1))
                    {
                        return Err("unbound named callback requester".into());
                    }
                }
            }
        }
        Ok(())
    }
}
