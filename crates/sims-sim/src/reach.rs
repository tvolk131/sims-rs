//! Native Reach timing/transfer phases. Animator skin loading is supplied by the
//! linked content program; arbitrary placement callbacks remain explicit boundaries.
use crate::{Event, Simulation, animation::Playback};
use sims_vm::{
    Program, ThreadId,
    reach::{Request, animation_index, transition},
};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Callback {
    pub request: Request,
    pub thread: ThreadId,
    pub object: i16,
    pub owner: String,
    pub bhav: u16,
    pub depth: usize,
}
impl Simulation {
    pub(crate) fn drive_reach_callback(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        while let Some(e) = self.reach_callback.clone() {
            if !self.drive_cancellation(p, remaining, step)? {
                return Ok(false);
            }
            let t = &self.vm.threads[&e.thread];
            if t.callback.is_none() {
                t.callback_result.ok_or("Reach callback result missing")?;
                if !matches!(&t.status,sims_vm::Status::AwaitingReach(r) if r==&e.request) {
                    return Err("Reach callback resume mismatch".into());
                }
                self.vm
                    .threads
                    .get_mut(&e.thread)
                    .unwrap()
                    .stack
                    .last_mut()
                    .unwrap()
                    .continuation = 2;
                if self.native_lot.is_some() {
                    self.native_reaches
                        .get_mut(&e.request.id)
                        .ok_or("native Reach continuation absent")?
                        .callback_finished(self.vm.tick)?;
                } else {
                    self.animations
                        .get_mut(&e.request.caller)
                        .ok_or("Reach foreground missing after callback")?
                        .last_yield_tick = Some(self.vm.tick);
                }
                self.reach_callback = None;
                continue;
            }
            if *remaining == 0 {
                return Ok(false);
            }
            match t.status.clone() {
                sims_vm::Status::Ready => {
                    let r = self.vm.run_slice(p, e.thread, *remaining)?;
                    *remaining -= r.trace.len();
                    step.trace.extend(r.trace);
                    self.publish_budget_statistics();
                }
                sims_vm::Status::AwaitingCreation(r) => {
                    if !self.service_creation(p, e.thread, r, remaining, step)? {
                        return Ok(false);
                    }
                }
                sims_vm::Status::AwaitingTransfer(r) => {
                    if self.native_lot.is_some() {
                        if !self.service_native_transfer(p, e.thread, &r, remaining, step)? {
                            return Ok(false);
                        }
                    } else {
                        self.service_transfer(p, e.thread, &r, &mut step.events)?;
                    }
                }
                sims_vm::Status::AwaitingPlacement(r) => {
                    if self.native_lot.is_some() {
                        let execution =
                            crate::lot_find_location::Execution::begin(self, p, e.thread, &r)?;
                        self.native_lot.as_mut().unwrap().find_location = Some(execution);
                        if !self.drive_native_find_location(p, remaining, step)? {
                            return Ok(false);
                        }
                    } else {
                        self.service_find_location(p, e.thread, &r, &mut step.events)?;
                    }
                }
                sims_vm::Status::AwaitingRemoval(r) => {
                    if !self.service_removal(p, e.thread, &r, remaining, step)? {
                        return Ok(false);
                    }
                }
                sims_vm::Status::Faulted(f) => {
                    return Err(format!("Reach callback failed: {}", f.message));
                }
                s => {
                    return Err(format!(
                        "Reach callback yielded (native error 0x44f): {s:?}"
                    ));
                }
            }
        }
        Ok(true)
    }
    pub(crate) fn validate_reaches(&self, p: &Program) -> Result<(), String> {
        if let Some(e) = &self.reach_callback {
            let t = self
                .vm
                .threads
                .get(&e.thread)
                .ok_or("saved Reach callback thread missing")?;
            let f = t
                .stack
                .get(e.depth)
                .ok_or("saved Reach callback frame missing")?;
            let parent = e
                .depth
                .checked_sub(1)
                .and_then(|i| t.stack.get(i))
                .ok_or("saved Reach caller missing")?;
            let (_, routine) = p.resolve(&parent.code_owner, parent.location.bhav)?;
            let ins = routine
                .body
                .instructions
                .get(parent.location.instruction)
                .ok_or("saved Reach instruction missing")?;
            let mode = i16::from_le_bytes(e.request.operands[..2].try_into().unwrap());
            let name = if mode == 0 { "CT - Grab" } else { "CT - Put" };
            if ![0, 1].contains(&mode)
                || e.request.operands[2..4] == [0, 0]
                || parent.continuation != 1
                || parent.location != e.request.origin
                || parent.caller != e.request.caller
                || parent.stack_object != e.request.target
                || ins.opcode != 47
                || ins.operands != e.request.operands
                || (mode == 0 && e.object != e.request.target)
                || self.vm.resolve_named_tree(p, e.object, name)? != Some((e.owner.clone(), e.bhav))
            {
                return Err("invalid saved Reach callback origin".into());
            }
            if f.caller!=e.request.caller || f.callee!=e.object || f.code_owner!=e.owner || f.location.bhav!=e.bhav
                || t.callback.as_ref().is_none_or(|b| !b.chain().any(|b| b.external && b.base_depth==e.depth && matches!(&b.resume_status,sims_vm::Status::AwaitingReach(r) if r==&e.request)))
                || if self.native_lot.is_some() {
                    self.native_reaches.get(&e.request.id).is_none_or(|n| n.thread != e.thread || n.request != e.request)
                } else {
                    self.animations.get(&e.request.caller).is_none_or(|a| !a.pickup || a.thread!=e.thread)
                }
            {return Err("invalid saved Reach callback binding".into());}
        }
        for (&id, t) in &self.vm.threads {
            if let sims_vm::Status::AwaitingReach(r) = &t.status
                && t.stack.last().unwrap().continuation != 0
                && self.native_lot.is_none()
                && self.animations.get(&r.caller).is_none_or(|a| {
                    a.thread != id
                        || !a.pickup
                        || a.completed
                        || a.interruptible
                        || a.looping
                        || a.reverse
                        || a.speed != 6
                })
            {
                return Err("invalid saved Reach foreground binding".into());
            }
            if let sims_vm::Status::AwaitingReach(r) = &t.status
                && self.native_lot.is_some()
                && t.stack.last().unwrap().continuation != 0
                && !self.native_reaches.contains_key(&r.id)
            {
                return Err("saved native Reach continuation absent".into());
            }
        }
        Ok(())
    }
    fn prepare_reach(
        &self,
        p: &Program,
        id: ThreadId,
        r: &Request,
    ) -> Result<Option<(Option<i16>, Option<(i16, u16)>, f32)>, String> {
        let mode = i16::from_le_bytes(r.operands[..2].try_into().unwrap());
        if mode == 2 {
            return Ok(Some((None, None, -1.)));
        }
        let Some(target) = self.vm.world.objects.get(&r.target) else {
            return Ok(None);
        };
        let (object, container) = match mode {
            0 => (Some(r.target), target.container),
            1 => {
                let f = self.vm.threads[&id].stack.last().unwrap();
                let arg = i16::from_le_bytes(r.operands[4..6].try_into().unwrap());
                let slot = *usize::try_from(arg)
                    .ok()
                    .and_then(|i| f.args.get(i))
                    .ok_or("Reach slot argument invalid (native error 0x08)")?;
                if slot < 0 {
                    return Ok(None);
                }
                (
                    self.vm.world.objects[&r.caller]
                        .slots
                        .first()
                        .copied()
                        .flatten(),
                    Some((r.target, slot as u16)),
                )
            }
            _ => return Ok(None),
        };
        let mut height = 0f32;
        let mut next = container;
        let mut seen = std::collections::BTreeSet::new();
        while let Some((parent, slot)) = next {
            if !seen.insert(parent) {
                return Err("cyclic Reach container".into());
            }
            let o = &self.vm.world.objects[&parent];
            if slot as usize >= o.slots.len() {
                return Ok(None);
            }
            // Person built-in bone slots need their native animated coordinates.
            if o.person.is_some() {
                return Err("Reach height from a person bone slot is unsupported".into());
            }
            let d = o
                .definition
                .as_ref()
                .ok_or("Reach container definition missing")?;
            let s = p.modules[&d.source]
                .slots
                .get(&d.slots)
                .and_then(|v| v.entries.iter().filter(|s| s.kind == 0).nth(slot as usize))
                .ok_or("Reach slot metadata missing")?;
            height += sims_vm::routing::containment_height(s.height, s.offset[2]).1;
            next = o.container;
        }
        // 0x43404d accumulates f32 slot heights; 0x454f29 scales in x87.
        height = (f64::from(height) * f64::from((1.0 / (2.0 * 6f64.sqrt())) as f32) * 3.) as f32;
        Ok(Some((object, container, height)))
    }
    pub(crate) fn service_reach(
        &mut self,
        p: &Program,
        id: ThreadId,
        r: &Request,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        let phase = self.vm.threads[&id].stack.last().unwrap().continuation;
        if phase != 0
            && self
                .animations
                .get(&r.caller)
                .is_some_and(|a| a.last_yield_tick == Some(self.vm.tick))
        {
            return Ok(false);
        }
        let mode = i16::from_le_bytes(r.operands[..2].try_into().unwrap());
        let transfer = r.operands[2..4] != [0, 0];
        let mut prepared = true;
        let mut start = true;
        let mut ended = false;
        let mut zero = false;
        let mut placed = false;
        if phase == 0 {
            let prep = self.prepare_reach(p, id, r)?;
            prepared = prep.is_some();
            if let Some((_, _, height)) = prep {
                let person = self.vm.world.objects[&r.caller]
                    .person
                    .as_ref()
                    .ok_or("Reach actor is not a person")?;
                let index = animation_index(height, person[&0]);
                if index < 0 {
                    start = false;
                } else {
                    let table = if person[&58] > 0 && person[&58] < 18 {
                        155
                    } else {
                        154
                    };
                    let name = p.modules[&p.globals]
                        .strings
                        .get(&table)
                        .and_then(|s| s.get(index as usize))
                        .ok_or("Reach animation table entry missing")?
                        .to_ascii_lowercase();
                    if name.is_empty() {
                        start = false;
                    } else {
                        let asset = p
                            .animations
                            .get(&name)
                            .ok_or_else(|| format!("Reach animation not loaded: {name}"))?;
                        crate::animation::validate_clip(&asset.clip, false)?;
                        self.animations.insert(
                            r.caller,
                            Playback {
                                interruptible: false,
                                pickup: true,
                                reverse: false,
                                clip: name.clone(),
                                thread: id,
                                started_tick: self.vm.tick,
                                last_tick: self.vm.tick,
                                frame_fifths: 0,
                                speed: 6,
                                next_event: 0,
                                pending: Default::default(),
                                delivered: 0,
                                numbered_events: 0,
                                substitute_events: 0,
                                event_count: 0,
                                last_yield_tick: None,
                                ended: false,
                                completed: false,
                                looping: false,
                                hands: [0; 2],
                            },
                        );
                        // The native pickup foreground suppresses the automatic
                        // carry layer immediately. Deferring this until the next
                        // host slice made instruction budgets change saved poses.
                        self.sync_carry(p)?;
                        self.event(events, id, format!("Reach started {name}"));
                    }
                }
            }
        } else {
            let animation = self
                .animations
                .get_mut(&r.caller)
                .ok_or("Reach foreground missing")?;
            ended = animation.ended;
            if phase == 1 {
                zero = animation.pending.iter().any(|e| *e == 0);
                animation.delivered += animation.pending.len();
                animation.pending.clear();
                if ended || zero {
                    let prep = self.prepare_reach(p, id, r)?;
                    prepared = prep.is_some();
                    if let Some((Some(object), container, _)) = prep
                        && transfer
                        && [0, 1].contains(&mode)
                    {
                        let parent = if mode == 0 {
                            Some((r.caller, 0))
                        } else {
                            container
                        };
                        let parent = parent.ok_or("Reach Put target missing")?;
                        let name = if mode == 0 { "CT - Grab" } else { "CT - Put" };
                        let position = self.body(parent.0)?.position;
                        placed = self.transfer_at(
                            p,
                            id,
                            r.caller,
                            object,
                            Some(parent),
                            position,
                            events,
                        )?;
                        if placed
                            && let Some((owner, bhav)) =
                                self.vm.resolve_named_tree(p, object, name)?
                        {
                            if self.reach_callback.is_some()
                                || self.vm.threads[&id].callback.is_some()
                            {
                                return Err("nested Reach callback unsupported".into());
                            }
                            let depth = self.vm.threads[&id].stack.len();
                            let args = self.vm.world.objects[&r.caller].temps.to_vec();
                            self.vm.begin_immediate_callback(
                                p,
                                id,
                                sims_vm::Start {
                                    owner: &owner,
                                    bhav,
                                    caller: r.caller,
                                    callee: object,
                                    stack_object: object,
                                    args,
                                    check: true,
                                },
                            )?;
                            self.reach_callback = Some(Callback {
                                request: r.clone(),
                                thread: id,
                                object,
                                owner,
                                bhav,
                                depth,
                            });
                            return Ok(true);
                        }
                    }
                }
            }
        }
        let (result, next) =
            transition(phase, mode, transfer, prepared, start, ended, zero, placed);
        if result == -1 {
            return Err("invalid Reach continuation".into());
        }
        if result != 2 {
            if phase == 2 || phase == 3 {
                self.animations.remove(&r.caller);
                self.sync_carry(p)?;
            }
            self.vm.finish_reach(p, id, r.id, result == 1)?;
            return Ok(true);
        }
        self.vm
            .threads
            .get_mut(&id)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .continuation = next;
        self.animations
            .get_mut(&r.caller)
            .ok_or("yielded Reach lacks foreground")?
            .last_yield_tick = Some(self.vm.tick);
        Ok(false)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_reach_transitions_and_height_selection() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/home-reach.json")).unwrap();
        for r in report["cases"].as_array().unwrap() {
            let c = &r["input"];
            let (result, phase) = transition(
                c["phase"].as_u64().unwrap() as u32,
                c["mode"].as_i64().unwrap() as i16,
                c["transfer"].as_bool().unwrap(),
                c["prepare"].as_bool().unwrap(),
                c["start"].as_bool().unwrap(),
                c["ended"].as_bool().unwrap(),
                c["events"].as_array().unwrap().iter().any(|v| v == 0),
                c["place"].as_bool().unwrap(),
            );
            assert_eq!(
                result as u32,
                r["original"]["result"].as_u64().unwrap() as u32,
                "{c}"
            );
            assert_eq!(
                phase,
                r["original"]["phase"].as_u64().unwrap() as u32,
                "{c}"
            );
        }
        for c in report["heights"].as_array().unwrap() {
            assert_eq!(
                animation_index(
                    c["height"].as_f64().unwrap() as f32,
                    c["posture"].as_i64().unwrap() as i16
                ),
                c["index"].as_i64().unwrap() as i16
            );
        }
    }
}
