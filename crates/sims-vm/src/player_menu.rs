//! Ordinary residential player menus: 4582ad/45859c/458705/458319/4582dd.
//! Checks use the person's existing frame arena and live world. The caller owns
//! the paused UI operation; a diagnostic budget never advances household time.
//! Available active-action joins remain an explicit boundary. Dynamic entries
//! belong to the current check and suppress its ordinary entry even if it fails.
use crate::{ObjectId, Program, Run, Start, Status, Stop, ThreadId, Vm, action::Action};
use serde::{Deserialize, Serialize};

///4582dd's deferred destructor. The first query outlives the same-tile scan;
///each nested query has its own ordinary destructor and saved Temps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cleanup {
    actor: ObjectId,
    temps: Option<[i16; 8]>,
}
impl Cleanup {
    pub fn actor(&self) -> ObjectId {
        self.actor
    }
    pub fn restore(self, vm: &mut Vm) -> Result<(), String> {
        if let Some(temps) = self.temps {
            vm.world
                .objects
                .get_mut(&self.actor)
                .ok_or("player-menu cleanup actor absent")?
                .temps = temps;
        }
        Ok(())
    }
}
pub struct Completed {
    pub actions: Vec<Action>,
    pub cleanup: Cleanup,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Check {
    action: Action,
    owner: String,
    routine: u16,
    depth: usize,
    /// Older snapshots could not publish dynamic entries; use their current
    /// output length as the start when resuming that previously empty path.
    #[serde(default)]
    output_start: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Query {
    actor: ObjectId,
    target: ObjectId,
    thread: ThreadId,
    tick: u64,
    count: usize,
    next: usize,
    saved_temps: Option<[i16; 8]>,
    waiting: Option<Check>,
    actions: Vec<Action>,
}
impl Query {
    pub(crate) fn allows_run(&self, vm: &Vm, thread: ThreadId) -> bool {
        self.thread == thread
            && self.waiting.as_ref().is_some_and(|c| {
                vm.threads.get(&thread).is_some_and(|t| {
                    t.callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .any(|b| b.base_depth == c.depth)
                })
            })
    }
    pub fn owns_callback(&self, thread: ThreadId, depth: usize) -> bool {
        self.thread == thread && self.waiting.as_ref().is_some_and(|c| c.depth == depth)
    }
    pub fn actor(&self) -> ObjectId {
        self.actor
    }
    pub fn target(&self) -> ObjectId {
        self.target
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        let t = vm
            .threads
            .get(&self.thread)
            .ok_or("player-menu thread absent")?;
        if vm.tick != self.tick
            || vm.object_threads.get(&self.actor) != Some(&self.thread)
            || vm
                .world
                .actions
                .as_ref()
                .is_none_or(|s| !s.actors.contains_key(&self.actor))
            || vm
                .world
                .objects
                .get(&self.actor)
                .is_none_or(|o| o.person.is_none())
            || !vm.world.objects.contains_key(&self.target)
            || self.next > self.count
            || self.count > vm.player_menu_table(p, self.target)?.len()
            || self.actions.len() > 65536
            || self.waiting.is_none() && t.callback.is_some()
        {
            return Err("invalid player-menu continuation".into());
        }
        let valid_action = |a: &Action| {
            a.id == 0
                && a.target == self.target
                && a.icon_owner == self.target
                && a.priority == 50
                && (a
                    .label
                    .as_ref()
                    .is_some_and(|s| s.len() <= 65536 && !s.contains('\0'))
                    || (a.label.is_none() && a.args == [0; 4]))
                && a.flags & !(1 | 4 | 8 | 16) == 0
                && vm.player_menu_table(p, self.target).is_ok_and(|table| {
                    table[..self.next].iter().any(|e| {
                        e.id == a.menu_id
                            && e.action == a.routine
                            && a.flags & 1 == u32::from(e.flags & 0x100 != 0)
                    })
                })
        };
        if self
            .actions
            .iter()
            .any(|a| !valid_action(a) || a.flags & 28 != 12)
        {
            return Err("invalid published player-menu action".into());
        }
        if let Some(c) = &self.waiting {
            let entry = self
                .next
                .checked_sub(1)
                .and_then(|i| vm.player_menu_table(p, self.target).ok()?.get(i))
                .ok_or("player-menu check entry absent")?;
            let owner = &vm.world.objects[&self.target]
                .definition
                .as_ref()
                .ok_or("player-menu target definition absent")?
                .source;
            if !valid_action(&c.action)
                || c.action.flags & 28 != 4
                || self.saved_temps.is_none()
                || c.output_start.is_some_and(|n| n > self.actions.len())
                || entry.id != c.action.menu_id
                || entry.check == 0
                || entry.check != c.routine
                || *owner != c.owner
            {
                return Err("invalid player-menu check binding".into());
            }
            if t.callback
                .iter()
                .flat_map(|b| b.chain())
                .any(|b| b.base_depth == c.depth)
            {
                let f = t.stack.get(c.depth).ok_or("player-menu frame absent")?;
                if f.code_owner != c.owner
                    || f.location.bhav != c.routine
                    || f.caller != self.actor
                    || f.callee != self.target
                    || !t
                        .callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .any(|b| b.base_depth == c.depth && b.external)
                {
                    return Err("player-menu callback ownership mismatch".into());
                }
            } else if t.stack.len() != c.depth || t.callback_result.is_none() {
                return Err("player-menu callback result absent".into());
            }
        }
        Ok(())
    }
    ///458789 copies the in-progress base action, preserves its identity and
    ///priority, replaces its text/arguments, clears disabled and sets checked
    ///and available. It appends immediately, before the check returns.
    pub(crate) fn publish(
        &mut self,
        thread: ThreadId,
        label: String,
        args: [i16; 4],
    ) -> Result<(), String> {
        if thread != self.thread {
            return Err("dynamic menu publisher has a different thread".into());
        }
        if label.len() > 65536 || label.contains('\0') {
            return Err("dynamic menu label exceeds supported string storage".into());
        }
        let check = self
            .waiting
            .as_mut()
            .ok_or("dynamic menu publisher has no current check")?;
        if self.actions.len() >= 65536 {
            return Err("dynamic menu entries exceed diagnostic limit".into());
        }
        check.output_start.get_or_insert(self.actions.len());
        let mut action = check.action.clone();
        action.label = Some(label);
        action.args = args;
        action.flags = (action.flags & !16) | 4 | 8;
        self.actions.push(action);
        Ok(())
    }
}

impl Vm {
    fn player_menu_table<'a>(
        &self,
        p: &'a Program,
        target: ObjectId,
    ) -> Result<&'a [crate::program::MenuEntry], String> {
        let d = self
            .world
            .objects
            .get(&target)
            .and_then(|o| o.definition.as_ref())
            .ok_or("player-menu definition absent")?;
        let m = p
            .modules
            .get(&d.source)
            .ok_or("player-menu module absent")?;
        // 457cb0 can return null: a target without a TTAB has an empty menu.
        Ok(m.object_interactions
            .get(&d.object)
            .map_or(&[], Vec::as_slice))
    }
    pub fn begin_player_menu(
        &mut self,
        p: &Program,
        actor: ObjectId,
        target: ObjectId,
    ) -> Result<(), String> {
        if self.player_menu.is_some()
            || !self.named_calls.is_empty()
            || self.threads.values().any(|t| t.callback.is_some())
        {
            return Err("player menu overlaps a synchronous callback".into());
        }
        let target = crate::multitile::interaction_target(&self.world, p, target)?;
        let person = self
            .world
            .objects
            .get(&actor)
            .and_then(|o| o.person.as_ref())
            .ok_or("player-menu actor is not a person")?;
        let species = *person.get(&65).ok_or("player-menu species absent")?;
        if self
            .world
            .actions
            .as_ref()
            .is_none_or(|s| !s.actors.contains_key(&actor))
        {
            return Err("player-menu action storage absent".into());
        }
        let thread = *self
            .object_threads
            .get(&actor)
            .ok_or("player-menu actor has no Main")?;
        let t = self.threads.get(&thread).ok_or("player-menu Main absent")?;
        if matches!(t.status, Status::Faulted(_)) {
            return Err("player-menu actor is faulted".into());
        }
        if actor != target
            && self.world.objects[&target].person.is_some()
            && self.autonomy_join_target(p, target)?.is_some()
        {
            return Err("player-menu person target requires native join enumeration".into());
        }
        let guid = self.world.objects[&target]
            .definition
            .as_ref()
            .unwrap()
            .guid;
        // Original normal-play pet gate; the executable's debug override is not
        // enabled by this API. Go Here is the special permitted target.
        let count = if species & 6 != 0 && guid != 0x7c4 {
            0
        } else {
            self.player_menu_table(p, target)?.len()
        };
        self.player_menu = Some(Query {
            actor,
            target,
            thread,
            tick: self.tick,
            count,
            next: 0,
            saved_temps: None,
            waiting: None,
            actions: vec![],
        });
        Ok(())
    }
    /// Returns the original ordinary action records once the query destructor
    /// has restored Temps. All other world, RNG and frame-arena writes survive.
    /// Unsupported host services retain the query and its pending callback.
    pub fn advance_player_menu(
        &mut self,
        p: &Program,
        budget: usize,
    ) -> Result<(Option<Vec<Action>>, Run), String> {
        let (completed, run) = self.advance_player_menu_retained(p, budget)?;
        let actions = if let Some(completed) = completed {
            completed.cleanup.restore(self)?;
            Some(completed.actions)
        } else {
            None
        };
        Ok((actions, run))
    }
    ///Complete the same query body while leaving4582dd to the enclosing caller.
    pub fn advance_player_menu_retained(
        &mut self,
        p: &Program,
        budget: usize,
    ) -> Result<(Option<Completed>, Run), String> {
        self.player_menu
            .as_ref()
            .ok_or("player menu absent")?
            .validate(self, p)?;
        let mut trace = vec![];
        let mut remaining = budget;
        loop {
            let q = self.player_menu.as_ref().unwrap();
            let (actor, target, thread) = (q.actor, q.target, q.thread);
            if let Some(check) = &q.waiting {
                let active = self.threads[&thread]
                    .callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .any(|b| b.base_depth == check.depth);
                if active {
                    if remaining == 0 {
                        return Ok((
                            None,
                            Run {
                                stop: Stop::Budget,
                                trace,
                            },
                        ));
                    }
                    // A saved query from the older engine may be stopped at
                    // this exact formerly unsupported host boundary. Retry
                    // its unexecuted opcode, not arbitrary script failures.
                    let old_boundary = matches!(&self.threads[&thread].status,
                        Status::Faulted(f) if f.message == "player-menu Change Action String requires native dynamic entry publication");
                    if old_boundary && crate::current(p, &self.threads[&thread])?.opcode == 0x32 {
                        self.threads.get_mut(&thread).unwrap().status = Status::Ready;
                    }
                    let status = &self.threads[&thread].status;
                    if !matches!(status, Status::Ready) {
                        return Err(format!(
                            "player-menu check requires host service: {status:?}"
                        ));
                    }
                    let run = self.run_slice(p, thread, remaining)?;
                    remaining -= run.trace.len();
                    trace.extend(run.trace);
                    if run.stop == Stop::ActionCallbacks {
                        return Ok((
                            None,
                            Run {
                                stop: run.stop,
                                trace,
                            },
                        ));
                    }
                    // Returning the check restores Main's prior status, which
                    // can be an animation/route wait. Only a still-active check
                    // owns a newly encountered host boundary.
                    if self.player_menu.as_ref().unwrap().allows_run(self, thread)
                        && !matches!(run.stop, Stop::Budget | Stop::Returned(_))
                    {
                        return Err(format!(
                            "player-menu check requires host service: {:?}; {:?}",
                            run.stop, self.threads[&thread].status
                        ));
                    }
                    continue;
                }
                let result = self
                    .threads
                    .get_mut(&thread)
                    .unwrap()
                    .callback_result
                    .take()
                    .ok_or("player-menu check return absent")?;
                let check = self.player_menu.as_mut().unwrap().waiting.take().unwrap();
                let output_start = check
                    .output_start
                    .unwrap_or(self.player_menu.as_ref().unwrap().actions.len());
                let mut action = check.action;
                if result {
                    action.flags |= 8;
                }
                if self.world.objects[&actor].data[&50] != 0 {
                    action.flags |= 16;
                }
                //458705 compares the output vector's size before/after the
                //entire check. Explicit publications survive false/disabled
                //results and replace, rather than duplicate, the base entry.
                if self.player_menu.as_ref().unwrap().actions.len() == output_start
                    && action.flags & 24 == 8
                {
                    self.player_menu.as_mut().unwrap().actions.push(action);
                }
                continue;
            }
            if q.next == q.count {
                if actor != target
                    && self.world.objects[&target].person.is_some()
                    && self.autonomy_join_target(p, target)?.is_some()
                {
                    return Err("player-menu person target requires native join enumeration".into());
                }
                let q = self.player_menu.take().unwrap();
                return Ok((
                    Some(Completed {
                        actions: q.actions,
                        cleanup: Cleanup {
                            actor,
                            temps: q.saved_temps,
                        },
                    }),
                    Run {
                        stop: Stop::Returned(true),
                        trace,
                    },
                ));
            }
            let e = self.player_menu_table(p, target)?[q.next].clone();
            let allowed = self.immediate_eligibility(actor, target, e.flags)?;
            // 45859c enumerates forward and constructs priority 0x32 records.
            let action = Action {
                target,
                icon_owner: target,
                menu_id: e.id,
                priority: 50,
                flags: u32::from(e.flags & 0x100 != 0) | 4,
                routine: e.action,
                ..Default::default()
            };
            if !allowed {
                self.player_menu.as_mut().unwrap().next += 1;
                continue;
            }
            if e.check == 0 {
                let q = self.player_menu.as_mut().unwrap();
                q.next += 1;
                q.actions.push(Action {
                    flags: action.flags | 8,
                    ..action
                });
                continue;
            }
            let owner = self.world.objects[&target]
                .definition
                .as_ref()
                .unwrap()
                .source
                .clone();
            let depth = self.threads[&thread].stack.len();
            let mut t = self.threads[&thread].clone();
            let pushed = self.try_push_callback_start(
                p,
                &mut t,
                Start {
                    owner: &owner,
                    bhav: e.check,
                    caller: actor,
                    callee: target,
                    stack_object: target,
                    args: crate::action::eligibility_arguments(false, 0, 0, e.check),
                    check: true,
                },
            )?;
            if pushed {
                t.callback.as_mut().unwrap().external = true;
            }
            self.threads.insert(thread, t);
            let q = self.player_menu.as_mut().unwrap();
            q.next += 1;
            q.saved_temps
                .get_or_insert(self.world.objects[&actor].temps);
            if pushed {
                q.waiting = Some(Check {
                    action,
                    owner,
                    routine: e.check,
                    depth,
                    output_start: Some(q.actions.len()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_dynamic_publications_preserve_complete_action_records() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/native-dynamic-menu.json"))
                .unwrap();
        for row in report["publications"].as_array().unwrap() {
            let c = &row["input"];
            let base = Action {
                target: 2,
                icon_owner: 2,
                menu_id: 7,
                priority: 50,
                routine: 4096,
                attenuation_bits: 0x3f000000,
                flags: c["flags"].as_u64().unwrap() as u32,
                label: Some("old label".into()),
                ..Default::default()
            };
            let mut query = Query {
                actor: 1,
                target: 2,
                thread: 1,
                tick: 0,
                count: 1,
                next: 1,
                saved_temps: Some([0; 8]),
                waiting: Some(Check {
                    action: base.clone(),
                    owner: "test".into(),
                    routine: 4097,
                    depth: 0,
                    output_start: None,
                }),
                actions: vec![],
            };
            for (n, label) in c["labels"].as_array().unwrap().iter().enumerate() {
                query
                    .publish(
                        1,
                        label.as_str().unwrap().into(),
                        serde_json::from_value(c["args"][n].clone()).unwrap(),
                    )
                    .unwrap();
                query = serde_json::from_slice(&serde_json::to_vec(&query).unwrap()).unwrap();
            }
            let expected: Vec<Action> =
                serde_json::from_value(row["original"]["records"].clone()).unwrap();
            assert_eq!(query.actions, expected);
            assert_eq!(query.waiting.unwrap().action, base);
        }
    }
}
