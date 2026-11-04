//! 45c9ba over native character layers and physical placement. Only animation
//! waits permit time to pass; asset, acceptance and callback work is synchronous.
use crate::{Simulation, Step, lot_acceptance, lot_links::Descriptor, lot_placement, pose};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm, reach::Request};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Stage {
    Prepare,
    Profile {
        selection: pose::reach::Selection,
        started: bool,
    },
    Asset(pose::reach::Selection),
    Waiting,
    Transfer,
    Callback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Transfer {
    object: ObjectId,
    destination: Descriptor,
    acceptance: lot_acceptance::live::Execution,
    placement: Option<lot_placement::live::Execution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    pub(crate) thread: ThreadId,
    pub(crate) request: Request,
    stage: Stage,
    yielded: Option<u64>,
    #[serde(default)]
    repoll_visit: bool,
    transfer: Option<Transfer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Ready,
    Waiting,
    Budget,
}

impl Execution {
    pub(crate) fn rearm_object_visit(&mut self, thread: ThreadId) {
        if self.thread == thread && self.stage == Stage::Waiting {
            self.repoll_visit = true;
        }
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.transfer.as_ref().is_some_and(|x| {
            x.acceptance.owns_callback(vm, p, thread, depth)
                || x.placement
                    .as_ref()
                    .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
        })
    }
    pub(crate) fn yielded(&self, thread: ThreadId, request: u64, tick: u64) -> bool {
        self.thread == thread
            && self.request.id == request
            && self.stage == Stage::Waiting
            && self.yielded == Some(tick)
    }
    pub(crate) fn synchronous(&self) -> bool {
        self.stage != Stage::Waiting
    }
    pub(crate) fn callback_finished(&mut self, tick: u64) -> Result<(), String> {
        if self.stage != Stage::Callback {
            return Err("native Reach callback stage changed".into());
        }
        self.stage = Stage::Waiting;
        self.yielded = Some(tick);
        self.repoll_visit = false;
        self.transfer = None;
        Ok(())
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program, key: u64) -> Result<(), String> {
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("native Reach thread absent")?;
        let bound = std::iter::once(&t.status)
            .chain(
                t.callback
                    .iter()
                    .flat_map(|c| c.chain())
                    .map(|c| &c.resume_status),
            )
            .any(|status| matches!(status, Status::AwaitingReach(r) if r == &self.request));
        let f = t
            .stack
            .iter()
            .rev()
            .find(|f| {
                f.location == self.request.origin
                    && f.caller == self.request.caller
                    && f.stack_object == self.request.target
            })
            .ok_or("native Reach origin frame absent")?;
        let (_, routine) = p.resolve(&f.code_owner, f.location.bhav)?;
        let ins = routine
            .body
            .instructions
            .get(f.location.instruction)
            .ok_or("native Reach PC absent")?;
        let resident = s
            .characters
            .get(&self.request.caller)
            .ok_or("native Reach character absent")?;
        if s.native_lot.is_none()
            || !bound
            || key != self.request.id
            || ins.opcode != 47
            || ins.operands != self.request.operands
            || match self.stage {
                Stage::Profile { selection, .. } => {
                    !selection.reload_profile
                        || resident.profile_dirty
                        || !(1..=10).contains(&selection.index)
                        || ![154, 155].contains(&selection.strings)
                }
                Stage::Asset(selection) => {
                    resident.profile_dirty
                        || !(1..=10).contains(&selection.index)
                        || ![154, 155].contains(&selection.strings)
                }
                _ => false,
            }
            || self.yielded.is_some_and(|tick| tick > s.vm.tick)
            || match self.stage {
                Stage::Prepare | Stage::Profile { .. } | Stage::Asset(_) => {
                    f.continuation != 0 || self.yielded.is_some()
                }
                Stage::Waiting => !(1..=3).contains(&f.continuation) || self.yielded.is_none(),
                Stage::Transfer | Stage::Callback => f.continuation != 1,
            }
            || matches!(
                self.stage,
                Stage::Waiting | Stage::Transfer | Stage::Callback
            ) && resident.controls.ordinary.pickup != 1
            || matches!(self.stage, Stage::Transfer | Stage::Callback) != self.transfer.is_some()
            || (self.stage == Stage::Callback)
                != s.reach_callback
                    .as_ref()
                    .is_some_and(|c| c.request == self.request)
        {
            return Err("invalid native Reach continuation binding".into());
        }
        if let Some(x) = &self.transfer {
            let mode = i16::from_le_bytes(self.request.operands[..2].try_into().unwrap());
            let parent = if mode == 0 {
                self.request.caller
            } else {
                self.request.target
            };
            if ![0, 1].contains(&mode)
                || self.request.operands[2..4] == [0, 0]
                || x.destination.container != Some(parent)
                || !x.destination.on_lot
                || mode == 0 && (x.object != self.request.target || x.destination.slot != 0)
                || x.acceptance.binding() != (x.object, x.destination.position, x.destination.level)
                || x.acceptance.container_binding() != Some((parent, x.destination.slot))
            {
                return Err("native Reach transfer binding changed".into());
            }
            x.acceptance.validate(&s.vm, p)?;
            if let Some(e) = &x.placement {
                if !x.acceptance.outcome().is_some_and(|o| o.accepted)
                    || e.operation.binding() != (x.object, Some(x.destination))
                {
                    return Err("native Reach placement binding changed".into());
                }
                e.validate(&s.vm, p, x.object)?;
            }
            if self.stage == Stage::Callback && x.placement.as_ref().is_none_or(|e| !e.complete) {
                return Err("native Reach callback precedes placement".into());
            }
        }
        Ok(())
    }
}

impl Simulation {
    fn prepare_live_reach(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: &Request,
    ) -> Result<Option<super::Prepared>, String> {
        let args = self.vm.threads[&thread]
            .stack
            .last()
            .ok_or("native Reach frame absent")?
            .args
            .clone();
        let links = &self
            .native_lot
            .as_ref()
            .ok_or("native Reach physical host absent")?
            .placement
            .links;
        super::prepare(
            &mut self.vm.world,
            links,
            r.caller,
            r.target,
            &args,
            &r.operands,
            |world, object, slot| {
                Ok(lot_acceptance::contained::slots(world, p, object)?[slot].offset[2])
            },
        )
    }
    fn reach_name(&self, p: &Program, selection: pose::reach::Selection) -> Option<String> {
        p.modules
            .get(&p.globals)?
            .strings
            .get(&selection.strings)?
            .get(selection.index as usize)
            .filter(|name| !name.is_empty())
            .map(|name| name.to_ascii_lowercase())
    }
    pub(crate) fn native_reach_asset_names(
        &self,
        p: &Program,
    ) -> std::collections::BTreeSet<String> {
        self.native_reaches
            .values()
            .filter_map(|e| match e.stage {
                Stage::Asset(selection) => self.reach_name(p, selection),
                _ => None,
            })
            .filter(|name| !p.animations.contains_key(name))
            .collect()
    }
    pub fn pending_reach_profiles(&self) -> Vec<(ObjectId, u64, bool)> {
        self.native_reaches
            .iter()
            .filter_map(|(&id, e)| match e.stage {
                Stage::Profile { started, .. } => Some((e.request.caller, id, started)),
                _ => None,
            })
            .collect()
    }
    pub fn begin_reach_profile(&mut self, object: ObjectId, request: u64) -> Result<(), String> {
        let e = self
            .native_reaches
            .get_mut(&request)
            .ok_or("native Reach profile absent")?;
        match &mut e.stage {
            Stage::Profile { started, .. } if e.request.caller == object && !*started => {
                *started = true;
                Ok(())
            }
            _ => Err("native Reach profile already started or rebound".into()),
        }
    }
    pub fn complete_reach_profile(&mut self, object: ObjectId, request: u64) -> Result<(), String> {
        let e = self
            .native_reaches
            .get_mut(&request)
            .ok_or("native Reach profile absent")?;
        match e.stage {
            Stage::Profile {
                mut selection,
                started: true,
            } if e.request.caller == object => {
                // 5888d3 reloads the profile's age word. 59dcd4 reads it only
                // after that service; the height/posture index was computed before.
                let age = self.vm.world.objects[&object]
                    .person
                    .as_ref()
                    .ok_or("Reach person absent after profile")?
                    .get(&58)
                    .copied()
                    .ok_or("Reach age absent after profile")?;
                selection.reread_age(age);
                e.stage = Stage::Asset(selection);
                Ok(())
            }
            _ => Err("native Reach profile is not pending".into()),
        }
    }
    pub(crate) fn service_native_reach(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: &Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Outcome, String> {
        if !self.native_reaches.contains_key(&r.id)
            && self
                .vm
                .threads
                .get(&thread)
                .and_then(|t| t.stack.last())
                .is_none_or(|f| f.continuation != 0)
        {
            return Err("native Reach requires its saved continuation; original in-flight reconstruction is unsupported".into());
        }
        let mut e = self.native_reaches.remove(&r.id).unwrap_or(Execution {
            thread,
            request: r.clone(),
            stage: Stage::Prepare,
            yielded: None,
            repoll_visit: false,
            transfer: None,
        });
        if e.thread != thread || e.request != *r {
            return Err("stale native Reach request".into());
        }
        let result = self.drive_live_reach(p, &mut e, remaining, step);
        if !matches!(result, Ok(Outcome::Ready)) || e.stage == Stage::Callback {
            self.native_reaches.insert(r.id, e);
        }
        result
    }
    pub(crate) fn drive_pending_native_reaches(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let work: Vec<_> = self
            .native_reaches
            .values()
            .filter(|e| e.synchronous() && e.stage != Stage::Callback)
            .map(|e| (e.thread, e.request.clone()))
            .collect();
        for (thread, request) in work.into_iter().rev() {
            if self.service_native_reach(p, thread, &request, remaining, step)? == Outcome::Budget {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn reach_wait(&mut self, e: &mut Execution, phase: u32) -> Result<Outcome, String> {
        self.vm
            .threads
            .get_mut(&e.thread)
            .ok_or("native Reach thread absent")?
            .stack
            .last_mut()
            .ok_or("native Reach frame absent")?
            .continuation = phase;
        e.stage = Stage::Waiting;
        e.yielded = Some(self.vm.tick);
        e.repoll_visit = false;
        e.transfer = None;
        Ok(Outcome::Waiting)
    }
    fn reach_return(
        &mut self,
        p: &Program,
        e: &Execution,
        result: bool,
    ) -> Result<Outcome, String> {
        self.vm.finish_reach(p, e.thread, e.request.id, result)?;
        Ok(Outcome::Ready)
    }
    fn drive_live_reach(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Outcome, String> {
        let r = e.request.clone();
        let transfer = r.operands[2..4] != [0, 0];
        let mode = i16::from_le_bytes(r.operands[..2].try_into().unwrap());
        if e.stage == Stage::Waiting && e.yielded == Some(self.vm.tick) && !e.repoll_visit {
            return Ok(Outcome::Waiting);
        }
        if e.stage == Stage::Prepare {
            let Some(prepared) = self.prepare_live_reach(p, e.thread, &r)? else {
                return self.reach_return(p, e, false);
            };
            let person = self.vm.world.objects[&r.caller]
                .person
                .as_ref()
                .ok_or("native Reach actor not a person")?;
            let resident = self
                .characters
                .get_mut(&r.caller)
                .ok_or("native Reach character absent")?;
            if resident.character.busy()
                || resident.refresh_tick.is_some()
                || resident.ordinary_update.is_some()
            {
                return Err("native Reach during pending character update".into());
            }
            let selection = pose::reach::prepare(
                f32::from_bits(prepared.height_bits),
                person[&0],
                person[&58],
                &mut resident.profile_dirty,
            );
            let Some(selection) = selection else {
                return self.reach_return(p, e, !transfer);
            };
            e.stage = if selection.reload_profile {
                Stage::Profile {
                    selection,
                    started: false,
                }
            } else {
                Stage::Asset(selection)
            };
        }
        if matches!(e.stage, Stage::Profile { .. }) {
            return Ok(Outcome::Budget);
        }
        if let Stage::Asset(selection) = e.stage {
            let name = self.reach_name(p, selection);
            if name
                .as_ref()
                .is_some_and(|name| !p.animations.contains_key(name))
            {
                return Ok(Outcome::Budget);
            }
            let resource = name.as_ref().map_or(pose::reach::Resource::Absent, |name| {
                pose::reach::Resource::Ready(&p.animations[name].clip)
            });
            let resident = self
                .characters
                .get_mut(&r.caller)
                .ok_or("native Reach character absent")?;
            let started = pose::reach::start_selected(
                &mut resident.character.skin.animator,
                &mut resident.controls,
                resident
                    .source
                    .as_ref()
                    .ok_or("native Reach skeleton absent")?,
                resource,
                1,
                self.vm.tick as i32,
            )?;
            if !started {
                return self.reach_return(p, e, !transfer);
            }
            self.event(
                &mut step.events,
                e.thread,
                format!("native Reach started {}", name.unwrap()),
            );
            return self.reach_wait(e, 1);
        }
        if e.stage == Stage::Waiting {
            let phase = self.vm.threads[&e.thread]
                .stack
                .last()
                .unwrap()
                .continuation;
            let resident = self
                .characters
                .get_mut(&r.caller)
                .ok_or("native Reach character absent")?;
            let mut zero = false;
            if phase == 1 {
                while let Some(event) = resident.character.skin.animator.pop_numbered_event() {
                    zero |= event == 0;
                }
            }
            let ended = pose::reach::finished(&resident.character.skin.animator)?;
            if matches!(phase, 2 | 3) {
                if !ended {
                    return self.reach_wait(e, phase);
                }
                let posture = self.vm.world.objects[&r.caller].person.as_ref().unwrap()[&0];
                pose::reach::finish(
                    &mut resident.character.skin.animator,
                    &mut resident.controls,
                    posture,
                    self.vm.tick as i32,
                )?;
                return self.reach_return(p, e, phase == 2);
            }
            if phase != 1 {
                return Err("invalid native Reach continuation".into());
            }
            if !ended && !zero {
                return self.reach_wait(e, 1);
            }
            let Some(prepared) = self.prepare_live_reach(p, e.thread, &r)? else {
                return self.reach_return(p, e, !transfer);
            };
            if !transfer {
                return self.reach_wait(e, 2);
            }
            if ![0, 1].contains(&mode) {
                return self.reach_wait(e, 1);
            }
            let Some(object) = prepared.object else {
                return self.reach_wait(e, 3);
            };
            let parent = if mode == 0 {
                r.caller
            } else {
                prepared
                    .container
                    .ok_or("native Reach Put container absent")?
            };
            let slot = if mode == 0 { 0 } else { prepared.slot };
            let at = self
                .vm
                .world
                .positions
                .get(&parent)
                .ok_or("native Reach parent position absent")?;
            let destination = Descriptor {
                on_lot: true,
                position: [at.x, at.y],
                level: at.level,
                container: Some(parent),
                slot,
            };
            let o = &self.vm.world.objects[&object];
            let d = o
                .definition
                .as_ref()
                .ok_or("native Reach definition absent")?;
            let words = p
                .modules
                .get(&d.source)
                .and_then(|m| m.object_definition_words.get(&d.object))
                .filter(|w| w.len() > 10)
                .ok_or("native Reach OBJD absent")?;
            if o.person.is_some() || words[10] != 0 || words[9] == 9 {
                return Err("native Reach subclass placement adapter required".into());
            }
            e.transfer = Some(Transfer {
                object,
                destination,
                acceptance: lot_acceptance::live::Execution::new_contained(
                    self, object, parent, slot, at.level,
                )?,
                placement: None,
            });
            e.stage = Stage::Transfer;
        }
        if e.stage == Stage::Transfer {
            let x = e.transfer.as_mut().ok_or("native Reach transfer absent")?;
            if x.placement.is_none() {
                if !self.drive_native_acceptance(p, &mut x.acceptance, remaining, step)? {
                    return Ok(Outcome::Budget);
                }
                if !x.acceptance.outcome().unwrap().accepted {
                    return self.reach_wait(e, 3);
                }
                x.placement = Some(lot_placement::live::Execution::new(
                    lot_placement::Operation::set(x.object, x.destination, true),
                ));
            }
            if !self.drive_live_placement(p, x.placement.as_mut().unwrap(), remaining, step)? {
                return Ok(Outcome::Budget);
            }
            let name = if mode == 0 { "CT - Grab" } else { "CT - Put" };
            if let Some((owner, bhav)) = self.vm.resolve_named_tree(p, x.object, name)? {
                if self.reach_callback.is_some() {
                    return Err("nested Reach callback unsupported".into());
                }
                let depth = self.vm.threads[&e.thread].stack.len();
                self.vm.begin_immediate_callback(
                    p,
                    e.thread,
                    sims_vm::Start {
                        owner: &owner,
                        bhav,
                        caller: r.caller,
                        callee: x.object,
                        stack_object: x.object,
                        args: self.vm.world.objects[&r.caller].temps.to_vec(),
                        check: true,
                    },
                )?;
                self.reach_callback = Some(crate::reach::Callback {
                    request: r,
                    thread: e.thread,
                    object: x.object,
                    owner,
                    bhav,
                    depth,
                });
                e.stage = Stage::Callback;
                return Ok(Outcome::Ready);
            }
            return self.reach_wait(e, 2);
        }
        Err("native Reach has an unserviced callback".into())
    }
}
