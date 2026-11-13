//! Native 0x45ad63 states 7/8. Floor acceptance and SetObject may execute
//! callbacks; every phase is saved so a diagnostic budget never repeats them.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Begin,
    Accept,
    Commit,
    Heading,
    Animator,
    Fields,
    Done,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Completion {
    phase: Phase,
    success: bool,
    result: i32,
    clear_ignored: bool,
}
pub trait Services {
    fn person(&self, index: i16) -> Result<i16, String>;
    fn set_person(&mut self, index: i16, value: i16) -> Result<(), String>;
    fn data(&self, index: i16) -> Result<i16, String>;
    fn set_data(&mut self, index: i16, value: i16) -> Result<(), String>;
    fn accept(&mut self) -> Result<Option<bool>, String>;
    fn commit(&mut self) -> Result<bool, String>;
    fn path(&self) -> Vec<[i32; 2]>;
    fn orient(&mut self, direction: u8) -> Result<(), String>;
    fn moving(&self) -> bool;
    fn movement_diagnostic(&mut self) -> Result<(), String>;
    fn update_movement(&mut self) -> Result<(), String>;
    fn depth(&self) -> usize;
    fn finish(&mut self, success: bool) -> Result<(), String>;
    fn finish_with_result(&mut self, success: bool, _result: i32) -> Result<(), String> {
        self.finish(success)
    }
}
impl Completion {
    pub fn new(success: bool, result: i32) -> Self {
        Self {
            phase: Phase::Begin,
            success,
            result,
            clear_ignored: false,
        }
    }
    pub fn outcome(&self) -> Option<(bool, i32)> {
        (self.phase == Phase::Done).then_some((self.success, self.result))
    }
    /// False is a budget suspension, not a simulation tick yield.
    pub fn drive(&mut self, h: &mut impl Services) -> Result<bool, String> {
        loop {
            match self.phase {
                Phase::Begin => {
                    if self.success {
                        let ignored = h.person(72)?;
                        self.clear_ignored = ignored != 0;
                        if self.clear_ignored {
                            h.set_person(73, ignored)?;
                        }
                        self.phase = Phase::Accept;
                    } else {
                        self.phase = Phase::Animator;
                    }
                }
                Phase::Accept => {
                    let Some(accepted) = h.accept()? else {
                        return Ok(false);
                    };
                    if accepted {
                        self.phase = Phase::Commit;
                    } else {
                        if self.clear_ignored {
                            h.set_person(73, 0)?;
                        }
                        if h.person(0)? == 0 {
                            self.success = false;
                            self.result = 1;
                        }
                        self.phase = Phase::Animator;
                    }
                }
                Phase::Commit => {
                    if !h.commit()? {
                        return Ok(false);
                    }
                    self.phase = Phase::Heading;
                }
                Phase::Heading => {
                    let path = h.path();
                    if path.len() > 1 && h.data(9)? == 0 {
                        let direction =
                            sims_vm::routing::direction(path[path.len() - 2], path[path.len() - 1]);
                        h.orient((direction & 7) as u8)?;
                    }
                    self.phase = Phase::Animator;
                }
                Phase::Animator => {
                    if h.moving() {
                        h.movement_diagnostic()?;
                        h.update_movement()?;
                    }
                    self.phase = Phase::Fields;
                }
                Phase::Fields => {
                    self.result = if self.success {
                        0
                    } else if self.result == 0 {
                        1
                    } else {
                        self.result
                    };
                    h.set_person(62, self.result as i16)?;
                    if h.depth() == 1 && h.person(38)? == 0 {
                        h.set_person(39, 1)?;
                    }
                    h.set_data(9, 0)?;
                    h.finish_with_result(self.success, self.result)?;
                    self.phase = Phase::Done;
                }
                Phase::Done => return Ok(true),
            }
        }
    }
}

use crate::{Simulation, Step, lot_acceptance, lot_placement};
use sims_vm::{Program, RouteRequest, ThreadId, Vm};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    pub thread: ThreadId,
    request: RouteRequest,
    completion: Completion,
    acceptance: Option<lot_acceptance::live::Execution>,
    placement: Option<lot_placement::live::Execution>,
}
impl Execution {
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.acceptance
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if s.visit.map(|v| v.0) != Some(self.thread)
            || s.vm
                .threads
                .get(&self.thread)
                .and_then(|t| crate::contention::pending_route(t, self.request.id))
                != Some(&self.request)
            || s.route_control
                .get(&self.request.id)
                .is_none_or(|c| c.original.is_none() || c.caller != self.request.caller)
            || matches!(self.completion.phase, Phase::Done)
            || s.native_lot
                .as_ref()
                .is_none_or(|n| n.route.is_some() || n.movement.is_some())
        {
            return Err("invalid native route completion binding".into());
        }
        if let Some(e) = &self.acceptance {
            e.validate(&s.vm, p)?;
            if e.binding().0 != self.request.caller
                || e.container_binding().is_some()
                || self.completion.phase == Phase::Begin
            {
                return Err("invalid native completion acceptance".into());
            }
        }
        if let Some(e) = &self.placement {
            e.validate(&s.vm, p, self.request.caller)?;
            if !self
                .acceptance
                .as_ref()
                .and_then(|a| a.outcome())
                .is_some_and(|o| o.accepted)
                || !matches!(
                    self.completion.phase,
                    Phase::Commit | Phase::Heading | Phase::Animator | Phase::Fields
                )
                || e.operation
                    .binding()
                    .1
                    .is_none_or(|d| d.container.is_some() || d.slot != 0)
            {
                return Err("native completion placement before acceptance".into());
            }
        }
        if self.completion.phase == Phase::Accept && self.acceptance.is_none()
            || self.completion.phase == Phase::Commit && self.placement.is_none()
        {
            return Err("native completion lost synchronous continuation".into());
        }
        Ok(())
    }
}
struct Host<'a> {
    s: &'a mut Simulation,
    p: &'a Program,
    thread: ThreadId,
    request: &'a RouteRequest,
    acceptance: &'a mut Option<lot_acceptance::live::Execution>,
    placement: &'a mut Option<lot_placement::live::Execution>,
    remaining: &'a mut usize,
    step: &'a mut Step,
}
impl Services for Host<'_> {
    fn person(&self, i: i16) -> Result<i16, String> {
        self.s.vm.world.objects[&self.request.caller]
            .person
            .as_ref()
            .and_then(|p| p.get(&i))
            .copied()
            .ok_or("route completion person field absent".into())
    }
    fn set_person(&mut self, i: i16, v: i16) -> Result<(), String> {
        self.s.person(self.request.caller, i, v)
    }
    fn data(&self, i: i16) -> Result<i16, String> {
        self.s.vm.world.objects[&self.request.caller]
            .data
            .get(&i)
            .copied()
            .ok_or("route completion object field absent".into())
    }
    fn set_data(&mut self, i: i16, v: i16) -> Result<(), String> {
        self.s
            .vm
            .world
            .objects
            .get_mut(&self.request.caller)
            .unwrap()
            .data
            .insert(i, v);
        Ok(())
    }
    fn accept(&mut self) -> Result<Option<bool>, String> {
        if self.acceptance.is_none() {
            let q = self
                .s
                .vm
                .world
                .positions
                .get(&self.request.caller)
                .ok_or("completion position absent")?;
            *self.acceptance = Some(lot_acceptance::live::Execution::new(
                self.s,
                self.request.caller,
                [q.x, q.y],
                q.level,
            )?);
        }
        let e = self.acceptance.as_mut().unwrap();
        if !self
            .s
            .drive_native_acceptance(self.p, e, self.remaining, self.step)?
        {
            return Ok(None);
        }
        Ok(Some(
            e.outcome()
                .ok_or("completion acceptance incomplete")?
                .accepted,
        ))
    }
    fn commit(&mut self) -> Result<bool, String> {
        if self.placement.is_none() {
            // Native re-reads the current physical position after acceptance.
            let q = self
                .s
                .vm
                .world
                .positions
                .get(&self.request.caller)
                .ok_or("completion position absent")?;
            *self.placement = Some(lot_placement::live::Execution::new(
                super::movement::commit_operation(self.request.caller, [q.x, q.y], q.level),
            ));
        }
        if !self.s.drive_live_placement(
            self.p,
            self.placement.as_mut().unwrap(),
            self.remaining,
            self.step,
        )? {
            return Ok(false);
        }
        self.s.update_original_person_room(self.request.caller)?;
        Ok(true)
    }
    fn path(&self) -> Vec<[i32; 2]> {
        self.s
            .native_lot
            .as_ref()
            .unwrap()
            .route_paths
            .get(&self.request.caller)
            .cloned()
            .unwrap_or_default()
    }
    fn orient(&mut self, d: u8) -> Result<(), String> {
        self.s.face(self.request.caller, 1 << d)
    }
    fn moving(&self) -> bool {
        if self.s.native_character_settings.is_some() {
            // 589ea1 reads the current animator status, including mutations
            // made by completion's acceptance/placement callbacks.
            self.s.characters[&self.request.caller]
                .controls
                .movement
                .state
                .status
                != 0
        } else {
            self.s.routes.contains_key(&self.thread)
        }
    }
    fn movement_diagnostic(&mut self) -> Result<(), String> {
        // 0x4544dc is a diagnostic VM-stack dump, without gameplay effects.
        Ok(())
    }
    fn update_movement(&mut self) -> Result<(), String> {
        if self.s.native_character_settings.is_some() {
            let r = self
                .s
                .characters
                .get_mut(&self.request.caller)
                .ok_or("route completion character absent")?;
            // Completion ignores the stop return value (45b76a), unlike the
            // earlier route states 9/10 which yield until stopping finishes.
            r.controls
                .movement
                .stop_route(&mut r.character.placement, &mut r.controls.ordinary)?;
        } else {
            self.s.routes.remove(&self.thread);
        }
        Ok(())
    }
    fn depth(&self) -> usize {
        self.s
            .route_control
            .values()
            .filter(|c| c.caller == self.request.caller && c.original.is_some())
            .count()
    }
    fn finish(&mut self, success: bool) -> Result<(), String> {
        self.s
            .complete_route(self.p, self.thread, self.request, success)?;
        self.s.event(
            &mut self.step.events,
            self.thread,
            format!("original-lot native route cleanup completed: {success}"),
        );
        Ok(())
    }
    fn finish_with_result(&mut self, success: bool, result: i32) -> Result<(), String> {
        let event = self.s.vm.route_terminal_observations().enabled().then(|| {
            let owner = self.s.vm.native_route_action_owner(self.thread, self.request);
            crate::observations::NativeRouteTerminalEvent {
                actor: self.request.caller,
                target: self.request.target,
                request: self.request.id,
                thread: self.thread,
                action_serial: owner.map(|owner| owner.0),
                action_invocation: owner.map(|owner| owner.1),
                tick: self.s.vm.tick,
                success,
                native_result: result,
                endpoint: self.s.vm.world.positions.get(&self.request.caller).copied(),
            }
        });
        self.finish(success)?;
        if let Some(event) = event {
            self.s.vm.record_native_route_terminal(event);
        }
        Ok(())
    }
}
impl Simulation {
    pub(crate) fn begin_native_route_completion(
        &mut self,
        thread: ThreadId,
        request: &RouteRequest,
        success: bool,
        result: i32,
    ) -> Result<(), String> {
        let n = self
            .native_lot
            .as_mut()
            .ok_or("native route world absent")?;
        if n.completion.is_some() {
            return Err("native route completion already active".into());
        }
        n.completion = Some(Execution {
            thread,
            request: request.clone(),
            completion: Completion::new(success, result),
            acceptance: None,
            placement: None,
        });
        Ok(())
    }
    pub(crate) fn drive_native_route_completion(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let Some(mut e) = self.native_lot.as_mut().and_then(|n| n.completion.take()) else {
            return Ok(true);
        };
        let result = e.completion.drive(&mut Host {
            s: self,
            p,
            thread: e.thread,
            request: &e.request,
            acceptance: &mut e.acceptance,
            placement: &mut e.placement,
            remaining,
            step,
        });
        if !result.as_ref().is_ok_and(|done| *done) {
            self.native_lot.as_mut().unwrap().completion = Some(e);
        }
        result
    }
    pub(crate) fn update_original_person_room(&mut self, caller: i16) -> Result<(), String> {
        let rooms = self
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap()
            .floors
            .iter()
            .flat_map(|f| f.metrics.keys().copied())
            .collect();
        let n = self.native_lot.as_mut().unwrap();
        n.room_effects.extend(n.membership.update(
            &self.vm.world,
            caller,
            &rooms,
            n.selected_person == Some(caller),
        )?);
        self.apply_original_room_effects()?;
        Ok(())
    }
}
