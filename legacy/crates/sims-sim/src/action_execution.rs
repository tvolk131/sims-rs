//! Borrowed facts from ordinary registry execution, not continuous Session coverage.
//!
//! Sinks must not panic or reenter. No unwind isolation is provided. Nothing is
//! stored in Simulation, VM, saves, clones or forks. A fresh sink after restore
//! cannot adopt preexisting frames by serial, depth or current object lookup.
//! Gaps precede uncovered owners (including resumed work and failing calls);
//! they are not terminal, cancellation, route, readiness or eligibility evidence.
//! FrameBound and Returned with an unavailable binding remain distinct facts.

use crate::{HostPollTrace, Simulation, Step, service_observation};
use sims_vm::{Program, Run, ThreadId, Vm, action_observation::Fact};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimExecutionEvent {
    Vm(Fact),
    Gap { owner: &'static str },
}

#[derive(Default)]
pub(crate) struct Observer<'callback> {
    sink: Option<&'callback mut dyn FnMut(SimExecutionEvent)>,
}

impl<'callback> Observer<'callback> {
    pub(crate) fn new(sink: &'callback mut dyn FnMut(SimExecutionEvent)) -> Self {
        Self { sink: Some(sink) }
    }

    pub(crate) fn gap(&mut self, owner: &'static str) {
        if let Some(sink) = &mut self.sink {
            sink(SimExecutionEvent::Gap { owner });
        }
    }

    pub(crate) fn run_slice(
        &mut self,
        vm: &mut Vm,
        program: &Program,
        thread: ThreadId,
        budget: usize,
    ) -> Result<Run, String> {
        match &mut self.sink {
            Some(sink) => vm.run_slice_with_action_observer(program, thread, budget, &mut |fact| {
                sink(SimExecutionEvent::Vm(fact))
            }),
            None => vm.run_slice(program, thread, budget),
        }
    }
}

impl Simulation {
    /// Observe only the ordinary VM owner inside the real registry scheduler.
    /// Uncovered callbacks and cleanup declare conservative owner-scoped gaps.
    /// The sink must not panic or reenter and is borrowed only for this call.
    pub fn run_current_tick_with_action_observer(
        &mut self,
        program: &Program,
        budget: usize,
        sink: &mut dyn FnMut(SimExecutionEvent),
    ) -> Result<Step, String> {
        self.run_current_tick_with_host_polls_and_action_observer(program, budget, None, sink)
    }

    /// Preserve native host-poll traces without changing registry visit ownership.
    /// Service-observer APIs remain separate; this does not replace their cuts.
    pub fn run_current_tick_with_host_polls_and_action_observer(
        &mut self,
        program: &Program,
        budget: usize,
        polls: Option<&mut Vec<HostPollTrace>>,
        sink: &mut dyn FnMut(SimExecutionEvent),
    ) -> Result<Step, String> {
        if self.native_scheduler.is_none() || self.fixture_driver.is_some() {
            return Err(if polls.is_some() {
                "original household tick requires the native registry scheduler"
            } else {
                "household execution requires the original registry scheduler; legacy snapshots cannot be silently upgraded"
            }.into());
        }
        self.run_scheduled_slice_with_action_observer(
            program,
            budget,
            Self::next_registry_thread,
            polls,
            &mut service_observation::Observer::default(),
            &mut Observer::new(sink),
        )
    }
}
