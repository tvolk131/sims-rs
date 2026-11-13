//! Opt-in synchronous metadata producers, not an action-completion verifier.
//!
//! No observer, journal, sequence counter, or frame binding is stored in a VM,
//! thread, or save. The caller owns the borrowed, infallible callback and any
//! bounded journal/overflow accounting. Callbacks must not panic or reenter.
//! Native serials wrap and are not lifetime identities. Frame exits deliberately
//! do not borrow the current active record's serial: nested dispatch can have
//! displaced it. Imported/unobserved frames have no recoverable serial binding.
//! Object handles describe registry resolution at the mutation, never a
//! retained lifetime binding on an old queue record or imported frame. Recycled
//! script IDs must not be adopted from these lookups or wrapping serials.
//!
//! `run_local_slice_observed` is the local interpreter, not `run_slice`'s named
//! call/accessory/menu/registry orchestration, and is crate-private so it cannot
//! bypass those public execution owners. Local delegated branches and action-use
//! entries share the producer paths. Lifecycle continuations emit coverage gaps.
//! Host completion, immediate
//! rollback, function-4 callback scheduling, and actual actor erasure remain
//! unsupported. General native-action verification remains unsupported.
//!
//! The local interpreter cannot be invoked through a public observed escape:
//! ```compile_fail
//! fn bypass(vm: &mut sims_vm::Vm, program: &sims_vm::Program, id: sims_vm::ThreadId) {
//!     vm.run_local_slice_observed(program, id, 1, &mut |_| {});
//! }
//! ```

use crate::{Definition, Frame, ObjectId, Program, Run, ThreadId, Vm, World, action::Action};
use std::collections::BTreeMap;
use sha2::{Digest, Sha256};

pub const GENERAL_NATIVE_ACTION_VERIFICATION_SUPPORTED: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    NoRegistryContext,
    RegistryStorageMissing,
    ObjectMissing,
    RegistryHandleMissing,
    ActorNotSupplied,
    FrameHasNoSerialBinding,
    ProgramNotSupplied,
    RoutineUnavailable,
    NativeRecordHasNoLifetimeBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence<T> {
    Known(T),
    Unavailable(Unavailable),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectIdentity {
    pub id: Option<ObjectId>,
    /// Current lookup only; not the origin lifetime of a saved frame/record.
    pub handle: Evidence<u32>,
    pub definition: Option<Definition>,
}

impl ObjectIdentity {
    fn unavailable(id: Option<ObjectId>, reason: Unavailable) -> Self {
        Self { id, handle: Evidence::Unavailable(reason), definition: None }
    }

    pub fn capture(world: &World, id: ObjectId) -> Self {
        let object = world.objects.get(&id);
        let handle = if object.is_none() {
            Evidence::Unavailable(Unavailable::ObjectMissing)
        } else if let Some(storage) = &world.scheduler_storage {
            storage.handle(id).map_or(
                Evidence::Unavailable(Unavailable::RegistryHandleMissing),
                Evidence::Known,
            )
        } else {
            Evidence::Unavailable(Unavailable::RegistryStorageMissing)
        };
        Self { id: Some(id), handle, definition: object.and_then(|o| o.definition.clone()) }
    }
}

/// Scalar native record metadata; cached display labels are not copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSnapshot {
    pub actor: ObjectIdentity,
    pub target: ObjectIdentity,
    pub icon_owner: ObjectIdentity,
    pub serial: u32,
    pub menu_id: u32,
    pub routine: u16,
    pub priority: i32,
    pub flags: u32,
    pub args: [i16; 4],
    pub attenuation_bits: u32,
    pub retained_lifetime_binding: Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFingerprint {
    pub thread: Option<ThreadId>,
    pub depth: usize,
    /// Exact pre-pop/entry frame, including PC, arena offset and opaque locals.
    pub frame: Frame,
    pub caller: ObjectIdentity,
    pub callee: ObjectIdentity,
    pub stack_object: ObjectIdentity,
    pub action_target: Option<ObjectIdentity>,
    /// Immutable loaded module hash, not a fabricated hash for synthetic inputs.
    pub source_content_hash: Option<String>,
    pub code_owner_content_hash: Option<String>,
    /// Derived from the exact loaded routine metadata/instructions, not an
    /// assertion of original-resource provenance for synthetic routines.
    pub routine_sha256: Evidence<[u8; 32]>,
    pub retained_lifetime_binding: Unavailable,
}

impl FrameFingerprint {
    pub(crate) fn capture(
        world: &World,
        program: Option<&Program>,
        thread: Option<ThreadId>,
        depth: usize,
        frame: &Frame,
    ) -> Self {
        let hash = |source: &str| program
            .and_then(|p| p.modules.get(source))
            .map(|m| m.content_hash.clone())
            .filter(|hash| !hash.is_empty());
        let routine_sha256 = match program {
            None => Evidence::Unavailable(Unavailable::ProgramNotSupplied),
            Some(program) => match program.resolve(&frame.code_owner, frame.location.bhav) {
                Err(_) => Evidence::Unavailable(Unavailable::RoutineUnavailable),
                Ok((_, routine)) => {
                    let mut digest = Sha256::new();
                    digest.update(b"sims-vm/action-observation/routine/v1");
                    digest.update((routine.name.len() as u64).to_le_bytes());
                    digest.update(routine.name.as_bytes());
                    digest.update(routine.body.format.to_le_bytes());
                    digest.update([routine.body.kind, routine.body.args]);
                    digest.update(routine.body.locals.to_le_bytes());
                    digest.update((routine.body.instructions.len() as u64).to_le_bytes());
                    for instruction in &routine.body.instructions {
                        digest.update(instruction.opcode.to_le_bytes());
                        digest.update([instruction.on_true, instruction.on_false]);
                        digest.update(instruction.operands);
                    }
                    Evidence::Known(digest.finalize().into())
                }
            },
        };
        Self {
            thread,
            depth,
            frame: frame.clone(),
            caller: ObjectIdentity::capture(world, frame.caller),
            callee: ObjectIdentity::capture(world, frame.callee),
            stack_object: ObjectIdentity::capture(world, frame.stack_object),
            action_target: frame.action_target.map(|id| ObjectIdentity::capture(world, id)),
            source_content_hash: hash(&frame.location.source),
            code_owner_content_hash: hash(&frame.code_owner),
            routine_sha256,
            retained_lifetime_binding: Unavailable::NativeRecordHasNoLifetimeBinding,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueTransition {
    SerialAssigned,
    InsertionAccepted,
    InsertionRejected,
    PreinsertError,
    CancelledBeforeStart,
    ReplacedBeforeStart,
    DiscardedBeforeStart,
    Dequeued,
    PrestartRejected,
    ReferenceRemoved { target: ObjectId },
    ReferenceScanDequeued { target: ObjectId },
    ResetRemoved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeReturn {
    TrueFe,
    FalseFf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearCause {
    Reset,
    ReferenceNotification { target: ObjectId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageGap {
    LifecycleContinuation,
    HostAndNamedOrchestration,
    ImmediateTransaction,
    ResetHostServices,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    Queue { action: ActionSnapshot, transition: QueueTransition },
    EnqueueFailed { actor: ObjectIdentity },
    ActiveCancelRequested { action: ActionSnapshot },
    ActiveCleared { action: ActionSnapshot, cause: ClearCause },
    FrameEntered { frame: FrameFingerprint, candidate: ActionSnapshot, serial_assignment_pending: bool },
    ActionUseEntered { frame: FrameFingerprint, binding: Evidence<u32> },
    FrameBound { frame: FrameFingerprint, action: ActionSnapshot },
    ActiveDisplaced { previous: ActionSnapshot, installed: ActionSnapshot },
    DispatchFailed { action: ActionSnapshot, frame_entered: Evidence<bool> },
    Returned {
        frame: FrameFingerprint,
        branch: NativeReturn,
        outermost_action_use: bool,
        binding: Evidence<u32>,
    },
    ResetUnwound {
        frame: FrameFingerprint,
        notification_target: Option<ObjectId>,
        binding: Evidence<u32>,
    },
    Gap(CoverageGap),
}

pub(crate) struct Observation<'a> {
    sink: Option<&'a mut dyn FnMut(Fact)>,
    pub(crate) thread: Option<ThreadId>,
}

impl<'a> Observation<'a> {
    pub(crate) fn disabled() -> Self {
        Self { sink: None, thread: None }
    }

    pub(crate) fn new(sink: &'a mut dyn FnMut(Fact), thread: Option<ThreadId>) -> Self {
        Self { sink: Some(sink), thread }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.sink.is_some()
    }

    pub(crate) fn emit(&mut self, fact: impl FnOnce() -> Fact) {
        if let Some(sink) = &mut self.sink {
            sink(fact());
        }
    }
}

#[derive(Default)]
pub(crate) struct Context {
    actor: Option<ObjectIdentity>,
    identities: BTreeMap<ObjectId, ObjectIdentity>,
}

impl Context {
    pub(crate) fn capture(world: &World, actor: ObjectId, input: Option<&Action>) -> Self {
        let mut context = Self { actor: Some(ObjectIdentity::capture(world, actor)), identities: BTreeMap::new() };
        let actions = world.actions.as_ref().and_then(|s| s.actors.get(&actor));
        for action in input.into_iter().chain(actions.into_iter().flat_map(|a| {
            a.queue.pending.iter().chain(a.active.iter()).chain(a.previous.iter())
        })) {
            for id in [action.target, action.icon_owner] {
                context.identities.entry(id).or_insert_with(|| ObjectIdentity::capture(world, id));
            }
        }
        context
    }

    pub(crate) fn actor(&self) -> ObjectIdentity {
        self.actor.clone().unwrap_or_else(|| ObjectIdentity::unavailable(None, Unavailable::ActorNotSupplied))
    }

    pub(crate) fn action(&self, action: &Action) -> ActionSnapshot {
        let identity = |id| self.identities.get(&id).cloned().unwrap_or_else(|| {
            ObjectIdentity::unavailable(Some(id), Unavailable::NoRegistryContext)
        });
        ActionSnapshot {
            actor: self.actor(), target: identity(action.target), icon_owner: identity(action.icon_owner),
            serial: action.id, menu_id: action.menu_id, routine: action.routine,
            priority: action.priority, flags: action.flags, args: action.args,
            attenuation_bits: action.attenuation_bits,
            retained_lifetime_binding: Unavailable::NativeRecordHasNoLifetimeBinding,
        }
    }

    pub(crate) fn emit(&self, observation: &mut Observation<'_>, action: &Action, transition: QueueTransition) {
        observation.emit(|| Fact::Queue { action: self.action(action), transition });
    }
}

impl Vm {
    /// Observes the actual local interpreter. This intentionally does not claim
    /// coverage of the public named-call/registry/host orchestration in run_slice.
    /// This is crate-private, like run_local_slice; external callers cannot
    /// bypass the public run_slice execution owners.
    pub(crate) fn run_local_slice_observed(
        &mut self,
        program: &Program,
        id: ThreadId,
        budget: usize,
        sink: &mut dyn FnMut(Fact),
    ) -> Result<Run, String> {
        let mut observation = Observation::new(sink, Some(id));
        observation.emit(|| Fact::Gap(CoverageGap::HostAndNamedOrchestration));
        self.run_local_slice_with_observation(program, id, budget, &mut observation)
    }
}

#[cfg(test)]
#[path = "action_observation_tests.rs"]
mod tests;
