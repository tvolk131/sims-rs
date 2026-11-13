//! Opt-in producer-local diagnostics, excluded from gameplay equality and saves.
use crate::{Frame, ObjectId, ThreadId, Vm, action::Action};
use std::collections::{BTreeMap, VecDeque};

/// Cloning deliberately creates a disabled, empty observer. Preview execution
/// must opt in independently and cannot publish the producer's evidence.
#[derive(Debug)]
pub struct ObservationBuffer<Event> {
    capacity: Option<usize>,
    events: VecDeque<Event>,
    dropped: u64,
}
impl<Event> Default for ObservationBuffer<Event> {
    fn default() -> Self {
        Self {
            capacity: None,
            events: VecDeque::new(),
            dropped: 0,
        }
    }
}
impl<Event> Clone for ObservationBuffer<Event> {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl<Event> PartialEq for ObservationBuffer<Event> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl<Event> Eq for ObservationBuffer<Event> {}
impl<Event> ObservationBuffer<Event> {
    fn checkpoint(&self) -> Self
    where
        Event: Clone,
    {
        Self {
            capacity: self.capacity,
            events: self.events.clone(),
            dropped: self.dropped,
        }
    }
    /// Starts a fresh observation session. Zero capacity counts every loss.
    pub fn enable(&mut self, capacity: usize) {
        *self = Self {
            capacity: Some(capacity),
            ..Self::default()
        };
    }
    pub fn disable(&mut self) {
        *self = Self::default();
    }
    pub fn enabled(&self) -> bool {
        self.capacity.is_some()
    }
    pub fn events(&self) -> &VecDeque<Event> {
        &self.events
    }
    /// Draining does not reset the session's cumulative dropped-evidence count.
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
    /// Cumulative lost events or dispatch bindings since enable. A nonzero
    /// count means the session's terminal evidence is incomplete.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn push(&mut self, event: Event) {
        if let Some(capacity) = self.capacity {
            if self.events.len() < capacity {
                self.events.push_back(event);
            } else {
                self.drop_evidence();
            }
        }
    }
    fn drop_evidence(&mut self) {
        self.dropped = self.dropped.saturating_add(1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionTerminalReason {
    NormalReturn,
    Reset,
    Replacement,
    Unqualified,
}

/// Retirement of an active record, not a queue/previous-record disappearance.
/// Only NormalReturn has an explicitly observed outer action return value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActionTerminalEvent {
    pub actor: ObjectId,
    pub serial: u32,
    pub target: ObjectId,
    pub thread: Option<ThreadId>,
    pub invocation: Option<u64>,
    pub tick: u64,
    pub reason: ActionTerminalReason,
    pub cancelled: bool,
    pub result: Option<bool>,
}

/// The host reports this only at native route final cleanup, using the actual
/// native result and placement before the resumed script can move the actor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NativeRouteTerminalEvent {
    pub actor: ObjectId,
    pub target: ObjectId,
    pub request: u64,
    pub thread: ThreadId,
    pub action_serial: Option<u32>,
    pub action_invocation: Option<u64>,
    pub tick: u64,
    pub success: bool,
    pub native_result: i32,
    pub endpoint: Option<crate::autonomy::Placement>,
}

#[derive(Debug, Clone)]
struct Binding {
    serial: u32,
    target: ObjectId,
    thread: ThreadId,
    invocation: u64,
    depth: usize,
    offset: usize,
    owner: String,
    routine: u16,
    outermost: bool,
    returned: Option<bool>,
}

#[derive(Debug, Default)]
pub(crate) struct ActionObserver {
    pub(crate) executing_thread: Option<ThreadId>,
    buffer: ObservationBuffer<ActionTerminalEvent>,
    routes: ObservationBuffer<NativeRouteTerminalEvent>,
    bindings: BTreeMap<ObjectId, Binding>,
    invocation: u64,
}
impl Clone for ActionObserver {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for ActionObserver {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl ActionObserver {
    fn checkpoint(&self) -> Self {
        Self {
            executing_thread: self.executing_thread,
            buffer: self.buffer.checkpoint(),
            routes: self.routes.checkpoint(),
            bindings: self.bindings.clone(),
            invocation: self.invocation,
        }
    }
    pub(crate) fn bind(
        &mut self,
        actor: ObjectId,
        action: &Action,
        thread: ThreadId,
        stack: &[Frame],
    ) {
        let Some(capacity) = self.buffer.capacity else {
            return;
        };
        let Some(invocation) = self.invocation.checked_add(1) else {
            self.buffer.drop_evidence();
            return;
        };
        self.invocation = invocation;
        if self.bindings.remove(&actor).is_some() {
            self.buffer.drop_evidence();
        }
        if self.bindings.len() >= capacity {
            self.buffer.drop_evidence();
            return;
        }
        let Some(frame) = stack.last() else {
            return;
        };
        self.bindings.insert(
            actor,
            Binding {
                serial: action.id,
                target: action.target,
                thread,
                invocation,
                depth: stack.len(),
                offset: frame.storage_offset,
                owner: frame.code_owner.clone(),
                routine: frame.location.bhav,
                outermost: !stack[..stack.len() - 1]
                    .iter()
                    .any(|frame| frame.action_target.is_some()),
                returned: None,
            },
        );
    }

    pub(crate) fn returned(
        &mut self,
        thread: Option<ThreadId>,
        frame: &Frame,
        depth: usize,
        value: bool,
        outermost: bool,
    ) {
        if let Some(binding) = self.bindings.get_mut(&frame.caller)
            && thread == Some(binding.thread)
            && binding.outermost
            && outermost
            && frame.action_target == Some(binding.target)
            && binding.depth == depth
            && binding.offset == frame.storage_offset
            && binding.owner == frame.code_owner
            && binding.routine == frame.location.bhav
        {
            binding.returned = Some(value);
        }
    }

    pub(crate) fn retire(
        &mut self,
        actor: ObjectId,
        action: &Action,
        tick: u64,
        reason: ActionTerminalReason,
    ) {
        if !self.buffer.enabled() {
            return;
        }
        let binding = match self.bindings.remove(&actor) {
            Some(binding) if binding.serial == action.id && binding.target == action.target => {
                Some(binding)
            }
            Some(_) => {
                self.buffer.drop_evidence();
                None
            }
            None => None,
        };
        let result = if reason == ActionTerminalReason::NormalReturn {
            binding.as_ref().and_then(|binding| binding.returned)
        } else {
            None
        };
        let reason = if reason == ActionTerminalReason::NormalReturn && result.is_none() {
            ActionTerminalReason::Unqualified
        } else {
            reason
        };
        self.buffer.push(ActionTerminalEvent {
            actor,
            serial: action.id,
            target: action.target,
            thread: binding.as_ref().map(|binding| binding.thread),
            invocation: binding.as_ref().map(|binding| binding.invocation),
            tick,
            reason,
            cancelled: action.flags & crate::action::CANCELLED != 0,
            result,
        });
    }
}

#[derive(Debug)]
pub struct TerminalObservationCheckpoint(ActionObserver);

impl Vm {
    /// Stages an authoritative transaction, including producer-local diagnostics.
    /// Promote only on success; speculative previews must use ordinary clone.
    pub fn clone_for_transaction(&self) -> Self {
        Self {
            terminal_observer: self.terminal_observer.checkpoint(),
            ..self.clone()
        }
    }
    /// Captures producer-local evidence for a transactional rollback, not a preview.
    pub fn terminal_observation_checkpoint(&self) -> TerminalObservationCheckpoint {
        TerminalObservationCheckpoint(self.terminal_observer.checkpoint())
    }
    /// Restores only diagnostics after authoritative state has been rolled back.
    pub fn restore_terminal_observation_checkpoint(
        &mut self,
        checkpoint: TerminalObservationCheckpoint,
    ) {
        self.terminal_observer = checkpoint.0;
    }
    /// Observe only future dispatches. Already-running or restored actions
    /// cannot acquire normal-return credit without a producer-local binding.
    pub fn enable_terminal_observations(&mut self, capacity: usize) {
        self.terminal_observer = ActionObserver::default();
        self.terminal_observer.buffer.enable(capacity);
        self.terminal_observer.routes.enable(capacity);
    }
    pub fn disable_terminal_observations(&mut self) {
        self.terminal_observer = ActionObserver::default();
    }
    pub fn terminal_observations(&self) -> &ObservationBuffer<ActionTerminalEvent> {
        &self.terminal_observer.buffer
    }
    pub fn drain_terminal_observations(&mut self) -> Vec<ActionTerminalEvent> {
        self.terminal_observer.buffer.drain()
    }
    pub fn route_terminal_observations(&self) -> &ObservationBuffer<NativeRouteTerminalEvent> {
        &self.terminal_observer.routes
    }
    pub fn drain_route_terminal_observations(&mut self) -> Vec<NativeRouteTerminalEvent> {
        self.terminal_observer.routes.drain()
    }
    /// Supplies ownership only while this exact pending request remains under
    /// its live producer-bound outer action. Restored, replaced, nested, and
    /// already-running-at-enable actions have no qualified ownership here.
    pub fn native_route_action_owner(
        &self,
        thread: ThreadId,
        request: &crate::RouteRequest,
    ) -> Option<(u32, u64)> {
        let state = self.threads.get(&thread)?;
        if !matches!(&state.status, crate::Status::AwaitingRoute(pending) if pending == request) {
            return None;
        }
        let active = self
            .world
            .actions
            .as_ref()?
            .actors
            .get(&request.caller)?
            .active
            .as_ref()?;
        let binding = self.terminal_observer.bindings.get(&request.caller)?;
        let root_index = binding.depth.checked_sub(1)?;
        let root = state.stack.get(root_index)?;
        let current = state.stack.last()?;
        if binding.thread != thread
            || binding.serial != active.id
            || binding.target != active.target
            || !binding.outermost
            || binding.returned.is_some()
            || root.caller != request.caller
            || root.action_target != Some(binding.target)
            || root.storage_offset != binding.offset
            || root.code_owner != binding.owner
            || root.location.bhav != binding.routine
            || current.caller != request.caller
            || current.location != request.origin
            || state.stack[root_index + 1..]
                .iter()
                .any(|frame| frame.action_target.is_some())
        {
            return None;
        }
        Some((binding.serial, binding.invocation))
    }
    /// Host seam for explicit native cleanup outcomes, not inferred arrivals.
    pub fn record_native_route_terminal(&mut self, event: NativeRouteTerminalEvent) {
        self.terminal_observer.routes.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Module, Object, Program, Routine, Start, Thread, World,
        action::{Actor, WorldState},
    };
    use sims_formats::behavior::{Behavior, Instruction};

    fn fixture() -> (Program, Vm, ThreadId) {
        let routine = Routine {
            name: "terminal observation".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![Instruction {
                    opcode: 0,
                    on_true: 254,
                    on_false: 255,
                    operands: [0; 8],
                }],
            },
        };
        let program = Program {
            globals: "fixture".into(),
            modules: [(
                "fixture".into(),
                Module {
                    source: "fixture".into(),
                    routines: [(4096, routine)].into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let action = Action {
            id: 17,
            target: 2,
            routine: 4096,
            ..Default::default()
        };
        let world = World {
            objects: [
                (
                    1,
                    Object {
                        person: Some(Default::default()),
                        ..Default::default()
                    },
                ),
                (
                    2,
                    Object {
                        data: [(62, 1)].into(),
                        ..Default::default()
                    },
                ),
            ]
            .into(),
            actions: Some(WorldState {
                actors: [(
                    1,
                    Actor {
                        active: Some(action.clone()),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut vm = Vm::new(world, 123);
        let thread = vm
            .spawn(
                &program,
                Start {
                    owner: "fixture",
                    bhav: 4096,
                    caller: 1,
                    callee: 2,
                    stack_object: 2,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.threads.get_mut(&thread).unwrap().stack[0].action_target = Some(2);
        vm.enable_terminal_observations(4);
        vm.terminal_observer
            .bind(1, &action, thread, &vm.threads[&thread].stack);
        (program, vm, thread)
    }

    fn returned(program: &Program, vm: &mut Vm, thread: ThreadId, value: bool) -> Thread {
        let mut state = vm.threads.remove(&thread).unwrap();
        vm.branch_in_thread(program, thread, &mut state, if value { 254 } else { 255 })
            .unwrap();
        state
    }

    #[test]
    fn terminal_observations_reject_disappearance_without_return_provenance() {
        let (_, mut vm, thread) = fixture();
        let mut state = vm.threads.remove(&thread).unwrap();
        state.stack.clear();
        state.completed_actions.push((1, 2, true, false, 1));
        vm.finish_action_frames(&mut state).unwrap();
        let events = vm.drain_terminal_observations();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, ActionTerminalReason::Unqualified);
        assert_eq!(events[0].result, None);
        assert!(
            vm.world.actions.as_ref().unwrap().actors[&1]
                .previous
                .is_some()
        );
    }

    #[test]
    fn terminal_observations_reset_is_not_a_normal_return_or_known_result() {
        let (_, mut vm, thread) = fixture();
        let mut state = vm.threads.remove(&thread).unwrap();
        state.last_result = true;
        vm.world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .get_mut(&1)
            .unwrap()
            .active
            .as_mut()
            .unwrap()
            .flags |= crate::action::CANCELLED;
        vm.pop_person_reset_frame(&mut state).unwrap();
        let events = vm.drain_terminal_observations();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, ActionTerminalReason::Reset);
        assert_eq!(events[0].result, None);
        assert!(events[0].cancelled);
        assert!(state.last_result);
    }

    #[test]
    fn terminal_observations_nested_action_use_frames_cannot_claim_outer_completion() {
        let (program, mut vm, thread) = fixture();
        let state = vm.threads.get_mut(&thread).unwrap();
        state.stack[0].continuation = 1;
        let mut nested = state.stack[0].clone();
        nested.storage_offset = crate::frame_words(0, 0);
        nested.continuation = 0;
        state.stack.push(nested);
        state
            .frame_memory
            .resize(crate::frame_words(0, 0) * 2, Some(0));
        vm.world.objects.get_mut(&2).unwrap().data.insert(62, 2);
        let active = vm
            .world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .get_mut(&1)
            .unwrap()
            .active
            .as_mut()
            .unwrap();
        active.id += 1;
        vm.terminal_observer.bind(1, active, thread, &state.stack);
        let mut state = returned(&program, &mut vm, thread, true);
        vm.finish_action_frames(&mut state).unwrap();
        assert_eq!(state.stack.len(), 1);
        assert!(vm.terminal_observations().events().is_empty());
        vm.branch_in_thread(&program, thread, &mut state, 254)
            .unwrap();
        vm.finish_action_frames(&mut state).unwrap();
        let events = vm.drain_terminal_observations();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, ActionTerminalReason::Unqualified);
        assert_eq!(events[0].result, None);
    }

    #[test]
    fn terminal_observations_reject_wrong_serial_target_and_thread() {
        for mismatch in ["serial", "target", "thread"] {
            let (program, mut vm, thread) = fixture();
            let action = vm
                .world
                .actions
                .as_mut()
                .unwrap()
                .actors
                .get_mut(&1)
                .unwrap()
                .active
                .as_mut()
                .unwrap();
            match mismatch {
                "serial" => action.id += 1,
                "target" => action.target = 1,
                _ => (),
            }
            let mut state = vm.threads.remove(&thread).unwrap();
            vm.branch_in_thread(
                &program,
                if mismatch == "thread" {
                    thread + 1
                } else {
                    thread
                },
                &mut state,
                254,
            )
            .unwrap();
            vm.finish_action_frames(&mut state).unwrap();
            let events = vm.drain_terminal_observations();
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].reason,
                ActionTerminalReason::Unqualified,
                "{mismatch}"
            );
            assert_eq!(events[0].result, None, "{mismatch}");
            assert_eq!(
                vm.terminal_observations().dropped(),
                u64::from(mismatch != "thread")
            );
        }
    }

    #[test]
    fn terminal_observations_preserve_retirement_cancellation_and_exact_return_value() {
        for value in [false, true] {
            let (program, mut vm, thread) = fixture();
            let mut state = returned(&program, &mut vm, thread, value);
            vm.tick = 71;
            vm.world
                .actions
                .as_mut()
                .unwrap()
                .actors
                .get_mut(&1)
                .unwrap()
                .active
                .as_mut()
                .unwrap()
                .flags |= crate::action::CANCELLED;
            vm.finish_action_frames(&mut state).unwrap();
            vm.finish_action_frames(&mut state).unwrap();
            let events = vm.drain_terminal_observations();
            assert_eq!(
                events,
                [ActionTerminalEvent {
                    actor: 1,
                    serial: 17,
                    target: 2,
                    thread: Some(thread),
                    invocation: Some(1),
                    tick: 71,
                    reason: ActionTerminalReason::NormalReturn,
                    cancelled: true,
                    result: Some(value),
                }]
            );
        }
    }

    #[test]
    fn terminal_observations_clone_and_deserialize_cannot_inherit_return_credit() {
        let (program, vm, thread) = fixture();
        for mut copy in [
            vm.clone(),
            serde_json::from_slice::<Vm>(&serde_json::to_vec(&vm).unwrap()).unwrap(),
        ] {
            assert_eq!(copy, vm);
            assert!(!copy.terminal_observations().enabled());
            copy.enable_terminal_observations(4);
            let mut state = returned(&program, &mut copy, thread, true);
            copy.finish_action_frames(&mut state).unwrap();
            assert_eq!(
                copy.drain_terminal_observations()[0].reason,
                ActionTerminalReason::Unqualified
            );
        }
        assert!(vm.terminal_observations().events().is_empty());
        assert_eq!(vm.terminal_observations().dropped(), 0);
    }

    #[test]
    fn terminal_observations_rejected_transaction_preserves_evidence_losses_and_live_binding() {
        let (program, mut vm, thread) = fixture();
        for request in 0..5 {
            vm.record_native_route_terminal(NativeRouteTerminalEvent {
                actor: 1,
                target: 2,
                request,
                thread,
                action_serial: Some(17),
                action_invocation: Some(1),
                tick: 0,
                success: false,
                native_result: 4,
                endpoint: None,
            });
        }
        let world = vm.world.clone();
        let threads = vm.threads.clone();
        let error = vm
            .push_route_service(&program, thread, "fixture".into(), 4096, 2, None)
            .unwrap_err();
        assert_eq!(error, "invalid portal entry boundary");
        assert_eq!(vm.world, world);
        assert_eq!(vm.threads, threads);
        assert!(vm.terminal_observations().enabled());
        assert!(vm.route_terminal_observations().enabled());
        assert_eq!(vm.route_terminal_observations().events().len(), 4);
        assert_eq!(vm.route_terminal_observations().dropped(), 1);
        assert_eq!(vm.terminal_observer.bindings[&1].serial, 17);
        assert!(!vm.clone().terminal_observations().enabled());
        let mut state = returned(&program, &mut vm, thread, true);
        vm.finish_action_frames(&mut state).unwrap();
        let terminal = vm.terminal_observations().events().back().unwrap();
        assert_eq!(terminal.reason, ActionTerminalReason::NormalReturn);
        assert_eq!(terminal.invocation, Some(1));
        assert_eq!(terminal.result, Some(true));
    }

    #[test]
    fn terminal_observations_staged_promotion_preserves_evidence_and_return_binding() {
        let (program, mut vm, thread) = fixture();
        for request in 0..5 {
            vm.record_native_route_terminal(NativeRouteTerminalEvent {
                actor: 1,
                target: 2,
                request,
                thread,
                action_serial: Some(17),
                action_invocation: Some(1),
                tick: 0,
                success: false,
                native_result: 4,
                endpoint: None,
            });
        }
        let mut staged = vm.clone_for_transaction();
        assert_eq!(staged, vm);
        assert!(staged.terminal_observations().enabled());
        assert!(staged.route_terminal_observations().enabled());
        assert_eq!(staged.route_terminal_observations().events().len(), 4);
        assert_eq!(staged.route_terminal_observations().dropped(), 1);
        assert!(!vm.clone().terminal_observations().enabled());
        let mut state = returned(&program, &mut staged, thread, true);
        staged.finish_action_frames(&mut state).unwrap();
        assert!(vm.terminal_observations().events().is_empty());
        vm = staged;
        let terminal = vm.terminal_observations().events().back().unwrap();
        assert_eq!(terminal.serial, 17);
        assert_eq!(terminal.reason, ActionTerminalReason::NormalReturn);
        assert_eq!(terminal.invocation, Some(1));
        assert_eq!(terminal.result, Some(true));
        assert_eq!(vm.route_terminal_observations().dropped(), 1);
    }

    #[test]
    fn terminal_observations_rejected_group_factory_preserves_producer_evidence() {
        let (mut program, mut vm, thread) = fixture();
        let module = program.modules.get_mut("fixture").unwrap();
        let mut catalog = vec![0; 108];
        catalog[10] = 7;
        catalog[11] = -1;
        let mut part = catalog.clone();
        part[11] = 0;
        part[63] = 1;
        module.object_definition_order = vec![0, 1];
        module.object_definition_words = [(0, catalog), (1, part)].into();
        for request in 0..5 {
            vm.record_native_route_terminal(NativeRouteTerminalEvent {
                actor: 1,
                target: 2,
                request,
                thread,
                action_serial: Some(17),
                action_invocation: Some(1),
                tick: 0,
                success: false,
                native_result: 4,
                endpoint: None,
            });
        }
        let before = vm.clone();
        let error = crate::group_factory::Factory::begin(
            &mut vm,
            &program,
            "fixture",
            0,
            Default::default(),
            [0, 0],
        )
        .unwrap_err();
        assert_eq!(error, "factory needs object registry and drawing storage");
        assert_eq!(vm, before);
        assert!(vm.terminal_observations().enabled());
        assert!(vm.route_terminal_observations().enabled());
        assert_eq!(vm.route_terminal_observations().events().len(), 4);
        assert_eq!(vm.route_terminal_observations().dropped(), 1);
        assert_eq!(vm.terminal_observer.bindings[&1].serial, 17);
        assert!(!before.terminal_observations().enabled());
        let mut state = returned(&program, &mut vm, thread, true);
        vm.finish_action_frames(&mut state).unwrap();
        let terminal = vm.terminal_observations().events().back().unwrap();
        assert_eq!(terminal.reason, ActionTerminalReason::NormalReturn);
        assert_eq!(terminal.invocation, Some(1));
        assert_eq!(terminal.result, Some(true));
    }

    #[test]
    fn terminal_observations_buffer_never_overwrites_and_counts_losses_across_drains() {
        let mut buffer = ObservationBuffer::default();
        buffer.push(1);
        assert_eq!(buffer.dropped(), 0);
        assert!(buffer.events().is_empty());
        buffer.enable(1);
        buffer.push(2);
        buffer.push(3);
        assert_eq!(buffer.drain(), [2]);
        assert_eq!(buffer.dropped(), 1);
        buffer.push(4);
        assert_eq!(buffer.drain(), [4]);
        assert_eq!(buffer.dropped(), 1);
        assert!(!buffer.clone().enabled());
        buffer.enable(0);
        buffer.push(5);
        assert_eq!(buffer.dropped(), 1);
        assert!(buffer.events().is_empty());
        buffer.disable();
        assert!(!buffer.enabled());
        assert_eq!(buffer.dropped(), 0);
    }
}
