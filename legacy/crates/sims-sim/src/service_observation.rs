//! Bounded, synchronous, read-only cuts of retained simulation service owners.
//!
//! Observers borrow the exact simulation at the cut; they are never saved or
//! called by nested service drivers. In particular, a creation or removal driver
//! may temporarily own a continuation outside `Simulation`. Its nested work is
//! deliberately unobserved, not reconstructed or exposed as a save boundary.
//! These cuts do not certify an application's surrounding save envelope. There
//! is no mid-inventory cut, scheduling yield, or engine reentry facility here.
//! Creation cuts cover ordinary and multipart object factories, not inferred
//! person-factory or inventory internals. Nested service calls stay unobserved.

use crate::{HostPollTrace, Simulation, Step};
use serde::Serialize;
use sims_vm::{ObjectId, Program, ThreadId};

/// An object ID plus its registered thread, when that lifetime is still present.
/// Neither a missing thread nor a reused object ID invents a lifetime identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ObjectLifetime {
    pub object: ObjectId,
    pub object_thread: Option<ThreadId>,
}

impl ObjectLifetime {
    pub(crate) fn at(simulation: &Simulation, object: ObjectId) -> Self {
        Self {
            object,
            object_thread: simulation.vm.object_threads.get(&object).copied(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CreationMoment {
    Registered,
    RotationInstalled,
    Reentry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CreationPhase {
    Factory,
    GroupFactory,
    GroupRotate,
    RotateOffLot,
    RotatePlaced,
    BeginPlacement,
    Check,
    Place,
    Dynamic,
    Detach,
    Destroy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ContinuationMoment {
    Installed,
    Reentry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FindLocationPhase {
    Search,
    Detach,
    DynamicDetach,
}

/// Actual resident service state, not an inferred instruction or future stage.
/// Request IDs and requester threads are retained where the owner supplies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ServiceCut {
    Creation {
        moment: CreationMoment,
        phase: CreationPhase,
        thread: ThreadId,
        request: u64,
        object: ObjectLifetime,
    },
    FindLocation {
        moment: ContinuationMoment,
        phase: FindLocationPhase,
        thread: ThreadId,
        request: u64,
        object: ObjectLifetime,
    },
    AwaitingAnimation {
        thread: ThreadId,
        request: u64,
        object: ObjectLifetime,
    },
    PrimitiveRemoval {
        moment: ContinuationMoment,
        thread: ThreadId,
        request: u64,
        object: ObjectLifetime,
    },
    /// A queue member immediately before the ordinary end-of-pass flush.
    /// The queue does not retain the originating primitive's request identity.
    /// This identifies the cut, not a serialized parent return position; the
    /// surrounding save envelope must still validate its own continuation.
    QueuedCleanup {
        index: usize,
        object: ObjectLifetime,
        suppress_reset: bool,
    },
}

impl ServiceCut {
    pub(crate) fn creation(
        simulation: &Simulation,
        moment: CreationMoment,
        phase: CreationPhase,
        thread: ThreadId,
        request: &sims_vm::create::Request,
    ) -> Self {
        Self::Creation {
            moment,
            phase,
            thread,
            request: request.id,
            object: ObjectLifetime::at(simulation, request.created.unwrap()),
        }
    }
}

pub(crate) type Callback<'callback> =
    dyn FnMut(ServiceCut, &Simulation) -> Result<(), String> + 'callback;

#[derive(Default)]
pub(crate) struct Observer<'callback> {
    callback: Option<&'callback mut Callback<'callback>>,
    failed: bool,
}

impl<'callback> Observer<'callback> {
    fn new(callback: &'callback mut Callback<'callback>) -> Self {
        Self {
            callback: Some(callback),
            failed: false,
        }
    }

    pub(crate) fn emit(&mut self, cut: ServiceCut, simulation: &Simulation) -> Result<(), String> {
        if let Some(callback) = &mut self.callback {
            if let Err(error) = callback(cut, simulation) {
                self.failed = true;
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed
    }
}

impl Simulation {
    /// Run the ordinary registry slice with synchronous borrowed service cuts.
    /// Callback errors abort the call unchanged, without rollback. The service
    /// owner remains installed at the failing cut. No callback enters a driver
    /// while an enclosing creation/removal continuation is borrowed out.
    pub fn run_current_tick_with_service_observer(
        &mut self,
        program: &Program,
        budget: usize,
        observer: &mut dyn FnMut(ServiceCut, &Simulation) -> Result<(), String>,
    ) -> Result<Step, String> {
        self.run_current_tick_with_host_polls_and_service_observer(program, budget, None, observer)
    }

    /// Service observation is independent of the existing host-poll trace sink.
    pub fn run_current_tick_with_host_polls_and_service_observer(
        &mut self,
        program: &Program,
        budget: usize,
        polls: Option<&mut Vec<HostPollTrace>>,
        observer: &mut dyn FnMut(ServiceCut, &Simulation) -> Result<(), String>,
    ) -> Result<Step, String> {
        if self.native_scheduler.is_none() || self.fixture_driver.is_some() {
            return Err("household execution requires the original registry scheduler; legacy snapshots cannot be silently upgraded".into());
        }
        self.run_scheduled_slice_observed(
            program,
            budget,
            Self::next_registry_thread,
            polls,
            &mut Observer::new(observer),
        )
    }

    /// The same bounded observer for explicitly authored laboratory fixtures.
    /// This never selects the fixture driver for an original household.
    pub fn run_fixture_tick_with_service_observer(
        &mut self,
        program: &Program,
        budget: usize,
        observer: &mut dyn FnMut(ServiceCut, &Simulation) -> Result<(), String>,
    ) -> Result<Step, String> {
        if self.native_scheduler.is_some() {
            return Err("fixture driver cannot run an original household".into());
        }
        self.run_scheduled_slice_observed(
            program,
            budget,
            Self::next_fixture_thread,
            None,
            &mut Observer::new(observer),
        )
    }
}

#[cfg(test)]
mod tests;
