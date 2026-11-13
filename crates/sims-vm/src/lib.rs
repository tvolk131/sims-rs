//! A deterministic, headless SimAntics interpreter. No wall clock, graphics, or async tasks.
//! Unsupported engine services stop at an explicit boundary; they never report success.
pub mod action;
pub mod animation;
pub mod autonomy;
pub mod autonomy_constants;
pub mod body_profiles;
pub mod budget;
pub mod burn;
pub mod career_appearance;
pub mod career_text;
pub mod commander;
pub mod compatibility;
pub mod construction;
pub mod create;
pub mod cursor;
pub mod deletion_entry;
pub mod dialog;
pub mod effect;
pub mod family;
pub mod family_label;
pub mod find_best;
pub mod fires;
pub mod functional;
pub mod global_load;
pub mod group_factory;
pub mod headline;
pub mod idle;
pub mod image_cache;
pub mod immediate;
pub mod indicator_pools;
pub mod indicator_refresh;
pub mod indicator_unregister;
pub mod interface;
pub mod lifecycle;
pub mod load_completion;
pub mod load_finalize;
pub mod load_indicators;
pub mod loading;
pub mod look;
pub mod magic_history;
mod memory;
pub mod motives;
pub mod multitile;
pub mod named;
pub mod native_suspension;
pub mod native_registry_reentry;
pub mod registry_suspension;
pub mod nearby_houses;
pub mod neighborhood;
pub mod numeric;
pub mod observations;
pub mod person;
pub mod person_factory;
mod person_friends;
mod family_friends;
pub mod person_loading;
pub mod person_reset;
pub mod placed_indicators;
pub mod placement;
pub mod player_menu;
pub mod presenter;
pub mod program;
pub mod reach;
pub mod refresh;
pub mod registry_flags;
pub mod relationship;
pub mod removal;
pub mod resource_storage;
pub mod retained_completed;
pub mod retained_epoch;
pub mod retained_frames;
pub mod rng;
pub mod routing;
pub mod slot_geometry;
pub mod scheduler;
pub mod selection;
pub mod selection_indicators;
mod snapshot;
pub mod transfer;
pub mod ui_effect;
pub mod ui_owner;
pub mod world_map;
pub mod zoning;
pub use program::{Module, Program, Routine};
use serde::{Deserialize, Serialize};
use sims_formats::behavior::Instruction;
pub use snapshot::Snapshot;
use std::collections::{BTreeMap, BTreeSet};

pub type ObjectId = i16;
pub type ThreadId = u64;
pub const MAX_STACK: usize = 128;
pub const MAX_VARIABLES: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Definition {
    pub source: String,
    pub object: u16,
    pub guid: u32,
    pub slots: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Object {
    /// TreeSim +0x28, read by 4745b2. Native construction initializes it to
    /// zero. A missing historical value is unknown, not a successful VM run;
    /// replacement-VM diagnostics are not native numeric error codes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_tree_error: Option<i16>,
    /// Native cXObject +0x114. Distinct from script data and registry scheduler
    /// flags. Older Rust snapshots/synthetic objects may not supply this word;
    /// do not infer it from the currently selected person or deletion queue.
    #[serde(default)]
    pub native_flags: Option<u32>,
    /// cXObject +0x188, read by 43458c. Only native construction supplies its
    /// initial zero; absent fields in historical/process snapshots stay unknown.
    /// This is distinct from the ordinary graphic owner's +0x180 readiness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_keep_dynamic: Option<u8>,
    /// Current person+aec curve cache; None in older saves is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_motive_scores: Option<motives::score_owner::Owner>,
    /// Current object+130 contained-slot geometry (physical slot zero omitted).
    /// None preserves unknown historical ownership; it is not an empty vector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_slot_geometry: Option<Vec<slot_geometry::Slot>>,
    pub routing_slots: Option<Vec<routing::Descriptor>>,
    pub relationships: relationship::Table,
    /// Native object+e4 row-vector order, independent of the keyed values.
    /// None in older snapshots means unknown order, not sorted-ID order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_order: Option<Vec<ObjectId>>,
    /// Ordered successfully loaded accessory appearances on the person's animator.
    pub accessories: Vec<String>,
    pub multitile: Option<multitile::Part>,
    /// cXCursorObject storage. Its multipart links belong to the cursor chain;
    /// its underlying furniture is a separate retained registry lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<cursor::State>,
    /// Host completion of the native object update, distinct from budget slices.
    pub last_update: Option<u64>,
    /// First native headline slot, when the presentation host allocates one.
    pub headline: Option<headline::State>,
    pub definition: Option<Definition>,
    pub attributes: Vec<i16>,
    /// Native object-owned registers, shared by its executions.
    pub temps: [i16; 8],
    pub dynamic_sprite_flags: Vec<i16>,
    /// Explicitly supplied storage fields. Missing engine-derived values are errors.
    pub data: BTreeMap<i16, i16>,
    pub person: Option<BTreeMap<i16, i16>>,
    /// Older replacement saves could not execute this helper at all. None
    /// retains that uninitialized host cache until its first selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub career_appearance: Option<career_appearance::State>,
    /// Original single-precision motive storage, serialized as exact IEEE-754 bits.
    pub motives: Option<[u32; 16]>,
    /// Native previous motives and decay clock belong to the person, even before a lot starts.
    pub motive_update: Option<motives::UpdateState>,
    /// Ordered native script deltas; duplicate motive indices are meaningful.
    pub motive_changes: Vec<motives::Change>,
    pub slots: Vec<Option<ObjectId>>,
    pub container: Option<(ObjectId, u16)>,
    /// Native object scheduler countdown; absent for host-only synthetic executions.
    pub idle: Option<idle::IdleState>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct World {
    /// One session-owned projection of original process tuning. Historical
    /// absence stays unknown across neighborhood/lot import and managed restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy_constants: Option<autonomy_constants::Owner>,
    ///Retained world+10. Absence in historical research saves means unknown
    ///resize history; neither global23 nor an array's allocation is this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_size: Option<world_map::State>,
    /// Registry +11 is disabled while44279a clears a lot. The default keeps
    /// historical managed sessions in their existing live presentation mode.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub registry_ui_disabled: bool,
    /// None is unknown process history, distinct from an observed null owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magic_history: Option<magic_history::Context>,
    /// Actual native registry extent/holes. Absence in older Rust snapshots is
    /// unknown history, not permission to rebuild it from the remaining IDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_storage: Option<scheduler::storage::Storage>,
    ///Registry-owned spatial cache. Absence in an old snapshot is unknown
    ///history; its separate presenter/physical records are never merged here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_order: Option<presenter::RegistryOrder>,
    /// Registry +ac/+b0/+bc, shared by person write intent, presenter refresh
    /// and synchronous object unregistration. No per-slice drain is implied.
    #[serde(
        default,
        skip_serializing_if = "indicator_unregister::Queues::is_empty"
    )]
    pub indicator_queues: indicator_unregister::Queues,
    /// Native view quadrant, shared by script depth queries and Look Towards.
    pub camera_rotation: u8,
    #[serde(default)]
    pub special_effects: effect::Context,
    pub resource_storage: resource_storage::State,
    /// Resource+0xb4 bit1, retained across lot reads. ObjT replaces this bit
    /// only for listed resources; fresh registrations start with it clear.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub changed_resources: BTreeSet<u32>,
    /// Presenter-owned image metadata; the headless/software view has no cached images.
    #[serde(default)]
    pub image_cache: image_cache::State,
    /// Definition caches and published in-memory BodyStrings banks are distinct.
    #[serde(default)]
    pub body_profiles: body_profiles::State,
    pub ui_owner: ui_owner::State,
    pub dialogs: dialog::State,
    #[serde(default)]
    pub fires: fires::State,
    /// Native horizontal subtile coordinates and level, published by the spatial host.
    pub positions: BTreeMap<ObjectId, autonomy::Placement>,
    /// Supplied room classification. Native architecture/tile derivation is not
    /// reconstructed by the authored room fixture.
    pub room_outdoors: BTreeMap<i16, bool>,
    /// Native room +0x38 triangle count; scope 23[3] returns half this value.
    pub room_half_tiles: BTreeMap<i16, i32>,
    pub budget: Option<budget::State>,
    pub objects: BTreeMap<ObjectId, Object>,
    pub globals: BTreeMap<i16, i16>,
    pub tree_advertisements: Vec<[i16; 3]>,
    pub neighborhood: BTreeMap<i16, i16>,
    pub refresh: Option<refresh::RefreshState>,
    pub actions: Option<action::WorldState>,
    pub person: Option<person::WorldState>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Location {
    pub source: String,
    pub bhav: u16,
    pub instruction: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Frame {
    /// Nonzero makes native returns resume this instruction instead of branching.
    pub continuation: u32,
    /// Native person action-use record for a frame entered through dispatch.
    pub action_target: Option<ObjectId>,
    pub action_icon: bool,
    /// Private object resource context survives global and semiglobal calls.
    pub code_owner: String,
    pub location: Location,
    pub caller: ObjectId,
    pub callee: ObjectId,
    pub stack_object: ObjectId,
    pub args: Vec<i16>,
    pub locals: Vec<i16>,
    /// Words inherited from native pointer storage, usable only after overwrite.
    pub opaque_locals: std::collections::BTreeSet<usize>,
    pub storage_offset: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fault {
    pub message: String,
    pub stack: Vec<Location>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteRequest {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub destination: routing::Destination,
    pub no_failure_trees: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapRequest {
    pub route: RouteRequest,
    pub flags: u8,
}
/// An animation instruction boundary, not a claim that its animation has been loaded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnimationRequest {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub operands: [u8; 8],
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRequest {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub mode: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SoundEvent {
    pub tick: u64,
    pub object: ObjectId,
    /// Ordered mixer command. Stop uses an empty name and accepts any native
    /// source ID, even if no corresponding object currently exists.
    pub stop: bool,
    pub name: String,
    pub flags: u8,
    pub sample_rate: u16,
    pub volume: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialEffect {
    pub tick: u64,
    pub object: ObjectId,
    pub duration_ms: i32,
    pub kind: u8,
    pub size: u8,
    pub flags: u8,
    #[serde(default)]
    pub text: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationOutcome {
    Completed,
    Event(i16),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Status {
    Ready,
    Sleeping {
        wake_tick: u64,
    },
    /// Source-bound ordinary native suspension; only a real registry Tick
    /// activates it. There is no imported future game-clock deadline.
    NativeSuspended(native_suspension::Owner),
    AwaitingRoute(RouteRequest),
    AwaitingSnap(SnapRequest),
    AwaitingAnimation(AnimationRequest),
    /// Host resource pause at the unchanged Change Suit instruction; not a VM yield.
    AwaitingAccessory {
        #[serde(default)]
        request: u64,
        origin: Location,
        caller: ObjectId,
        name: String,
    },
    AwaitingTurn(TurnRequest),
    AwaitingReach(reach::Request),
    AwaitingCreation(create::Request),
    AwaitingImmediate(immediate::Request),
    AwaitingTransfer(transfer::Request),
    AwaitingPlacement(placement::Request),
    AwaitingRemoval(removal::Request),
    AwaitingBurn(burn::Request),
    /// Synchronous dialog construction has not returned from its primitive.
    AwaitingDialog(Box<dialog::Construction>),
    AwaitingUiEffect(Box<ui_effect::Call>),
    Returned(bool),
    Faulted(Fault),
    /// Natural completed-registry owner. Appended to preserve old status ordinals.
    RegistrySuspended(registry_suspension::Owner),
    /// Broader typed completed-registry owner; old serialized variants unchanged.
    NativeRegistryReentry(native_registry_reentry::Owner),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallbackBoundary {
    pub external: bool,
    pub parent: Option<Box<CallbackBoundary>>,
    pub base_depth: usize,
    pub resume_status: Status,
    pub resume_check: bool,
}
impl CallbackBoundary {
    pub fn chain(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |b| b.parent.as_deref())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Thread {
    /// A yielding engine-service call on the existing stack, e.g. a room's Stand
    /// callback. The calling service consumes its return before Main continues.
    pub service: Option<lifecycle::ServiceCall>,
    pub dispatch: Option<action::Dispatch>,
    pub autonomy: Option<autonomy::Query>,
    pub find_best: Vec<find_best::Query>,
    pub completed_actions: Vec<(ObjectId, ObjectId, bool, bool, usize)>,
    pub last_result: bool,
    pub callback: Option<CallbackBoundary>,
    pub callback_result: Option<bool>,
    pub stack: Vec<Frame>,
    /// Reusable native frame arena in words. None marks opaque pointer storage.
    pub frame_memory: Vec<Option<i16>>,
    /// Explicit captured arena/context owner; absent for normal and saved imports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_frame_arena: Option<retained_frames::Arena>,
    pub check: bool,
    pub interrupted: bool,
    pub status: Status,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Vm {
    #[serde(skip)]
    pub(crate) terminal_observer: observations::ActionObserver,
    /// Explicit native object Update owner. Optional so historical authored
    /// profiles keep their existing per-tick scheduling until deliberately bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_object_visit: Option<scheduler::visit::Visit>,
    pub world: World,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_menu: Option<player_menu::Query>,
    /// Native characters own accessory records and geometry in the asset host.
    #[serde(default)]
    pub native_accessory_host: bool,
    pub tick: u64,
    /// Active native stream at0x6e54ac. Outside46d553 this is the global stream
    /// (SIMI's second word); an active lot_rng_scope temporarily installs the lot.
    pub rng_state: u32,
    /// Simulator-owned stream at+ac, SIMI's first RNG word. Remains unchanged
    /// during46d553; its working seed is published here when that call returns.
    pub lot_rng_state: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lot_rng_scope: Option<rng::LotScope>,
    /// CRT per-thread stream used by native default traits. A factory must
    /// supply it explicitly; it is not one of SIMI's two saved RNG words.
    pub crt_rng_state: Option<u32>,
    pub threads: BTreeMap<ThreadId, Thread>,
    pub object_threads: BTreeMap<ObjectId, ThreadId>,
    pub lifecycles: BTreeMap<ObjectId, lifecycle::Lifecycle>,
    pub named_calls: Vec<named::Call>,
    /// Presentation commands awaiting a client; headless execution does not play audio.
    pub sounds: Vec<SoundEvent>,
    /// Native Special Effect presentation requests; particles are client-owned.
    pub effects: Vec<SpecialEffect>,
    pub(crate) next_thread: u64,
    pub(crate) next_request: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trace {
    pub tick: u64,
    pub thread: ThreadId,
    pub depth: usize,
    pub location: Location,
    pub opcode: u16,
    pub operands: [u8; 8],
    pub effect: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Stop {
    ActionCallbacks,
    Returned(bool),
    Sleeping,
    AwaitingRoute,
    AwaitingSnap,
    AwaitingAnimation,
    AwaitingAccessory,
    AwaitingTurn,
    AwaitingReach,
    AwaitingCreation,
    AwaitingImmediate,
    AwaitingTransfer,
    AwaitingPlacement,
    AwaitingRemoval,
    AwaitingBurn,
    AwaitingDialog,
    AwaitingUiEffect,
    Faulted,
    Budget,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Run {
    pub stop: Stop,
    pub trace: Vec<Trace>,
}
pub struct Start<'a> {
    pub owner: &'a str,
    pub bhav: u16,
    pub caller: ObjectId,
    pub callee: ObjectId,
    pub stack_object: ObjectId,
    pub args: Vec<i16>,
    pub check: bool,
}
impl Vm {
    pub fn new(world: World, seed: u64) -> Self {
        Self {
            terminal_observer: Default::default(),
            world,
            scheduled_object_visit: None,
            player_menu: None,
            native_accessory_host: false,
            tick: 0,
            rng_state: seed as u32,
            lot_rng_state: None,
            lot_rng_scope: None,
            crt_rng_state: None,
            threads: BTreeMap::new(),
            object_threads: BTreeMap::new(),
            lifecycles: BTreeMap::new(),
            named_calls: vec![],
            sounds: Vec::new(),
            effects: Vec::new(),
            next_thread: 1,
            next_request: 1,
        }
    }
    pub fn spawn(&mut self, program: &Program, start: Start<'_>) -> Result<ThreadId, String> {
        if !self.world.objects.contains_key(&start.caller)
            || !self.world.objects.contains_key(&start.callee)
        {
            return Err("caller and callee must exist".into());
        }
        let (source, routine) = program.resolve(start.owner, start.bhav)?;
        let (argc, locals) = dimensions(routine)?;
        if start.args.len() > 255 {
            return Err("entry argument buffer exceeds native limit".into());
        }
        let entry = start.args.first().copied().unwrap_or(0);
        let mut args = start.args;
        args.resize(argc, 0);
        let frame = Frame {
            continuation: 0,
            action_target: None,
            action_icon: false,
            code_owner: start.owner.into(),
            location: Location {
                source: source.into(),
                bhav: start.bhav,
                instruction: entry_pc(routine, entry)?,
            },
            caller: start.caller,
            callee: start.callee,
            stack_object: start.stack_object,
            args,
            locals: vec![0; locals],
            opaque_locals: Default::default(),
            storage_offset: 0,
        };
        let id = self.next_thread;
        self.next_thread = id.checked_add(1).ok_or("thread ID exhausted")?;
        self.threads.insert(
            id,
            Thread {
                service: None,
                dispatch: None,
                autonomy: None,
                find_best: vec![],
                completed_actions: vec![],
                last_result: false,
                callback: None,
                callback_result: None,
                stack: vec![frame],
                frame_memory: vec![Some(0); frame_words(argc, locals)],
                retained_frame_arena: None,
                check: start.check,
                interrupted: false,
                status: Status::Ready,
            },
        );
        Ok(id)
    }
    /// Execute in object-ID order (thread ID breaks ties), with a diagnostic budget per thread.
    pub fn run_round(&mut self, program: &Program, budget: usize) -> Vec<(ThreadId, Run)> {
        // This infallible laboratory API already requires every selected thread
        // to be generically runnable. Fail before any thread/world effects.
        assert!(
            !self
                .threads
                .values()
                .any(|t| matches!(t.status, Status::NativeSuspended(_) | Status::RegistrySuspended(_)) || native_registry_reentry::contains(t)),
            "native suspensions require the original registry API"
        );
        if self.player_menu.is_some() {
            // Only advance_player_menu may publish/check the next menu entry.
            return vec![];
        }
        if let Some(call) = self.named_calls.first() {
            let id = call.requester;
            return vec![(
                id,
                self.run_slice(program, id, budget)
                    .expect("bound named requester"),
            )];
        }
        let mut ids: Vec<_> = self.threads.keys().copied().collect();
        ids.sort_by_key(|id| {
            (
                self.threads[id]
                    .stack
                    .first()
                    .map_or(i16::MAX, |f| f.caller),
                *id,
            )
        });
        let mut results = Vec::new();
        for id in ids {
            let run = self.run_slice(program, id, budget).expect("known thread");
            let exhausted = matches!(
                run.stop,
                Stop::Budget | Stop::AwaitingAccessory | Stop::AwaitingUiEffect
            ) || !self.named_calls.is_empty();
            results.push((id, run));
            // A diagnostic limit must not change cooperative scheduling order.
            if exhausted {
                break;
            }
        }
        results
    }
    fn validate_clock_boundary(&self) -> Result<(), String> {
        if self.lot_rng_scope.is_some() {
            return Err("lot RNG scope remains before game time can advance".into());
        }
        if self.scheduled_object_visit.is_some() {
            return Err("native object visit remains before game time can advance".into());
        }
        if self.player_menu.is_some() {
            return Err("player-menu query remains in the current tick".into());
        }
        if self
            .threads
            .values()
            .any(|t| matches!(t.status, Status::AwaitingUiEffect(_)))
        {
            return Err("UI-effect host call remains in the current tick".into());
        }
        if self.world.dialogs.modal.is_some()
            || self.world.dialogs.records.iter().any(|r| r.status == 0)
        {
            return Err("Dialog presentation remains in the current tick".into());
        }
        if self
            .threads
            .values()
            .any(|t| matches!(t.status, Status::AwaitingAccessory { .. }))
        {
            return Err("accessory resource work remains in the current tick".into());
        }
        if !self.named_calls.is_empty() {
            return Err("synchronous named callback remains in the current tick".into());
        }
        Ok(())
    }
    pub fn advance_tick(&mut self) -> Result<(), String> {
        if self
            .threads
            .values()
            .any(|t| matches!(t.status, Status::NativeSuspended(_) | Status::RegistrySuspended(_)) || native_registry_reentry::contains(t))
        {
            return Err("native suspension requires the original registry pass".into());
        }
        self.validate_clock_boundary()?;
        if self.threads.values().any(|t| {
            t.status == Status::Ready
                || matches!(t.status, Status::Sleeping { wake_tick } if wake_tick <= self.tick)
        }) {
            return Err(
                "runnable work remains in the current tick; resume it before advancing time".into(),
            );
        }
        for (&object, &id) in &self.object_threads {
            if let Some(idle) = self.world.objects[&object].idle.as_ref() {
                if idle.countdown > 0 && idle.last_run != Some(self.tick) {
                    return Err("scheduled object work remains in the current tick".into());
                }
            }
            if !self.threads.contains_key(&id) {
                return Err("missing object thread".into());
            }
        }
        self.tick = self.tick.checked_add(1).ok_or("tick counter exhausted")?;
        Ok(())
    }
    /// A completed native registry pass owns eligibility. Disabled entries may
    /// remain Ready and positive counters need not be visited in a partial pass.
    /// The simulation host must finish every requested Update before calling.
    pub fn finish_registry_pass(&mut self, pass: &scheduler::Pass) -> Result<bool, String> {
        pass.validate()?;
        let complete = pass
            .result()
            .ok_or("native registry pass has not returned")?;
        self.validate_clock_boundary()?;
        if self.world.scheduler_storage.is_none() {
            return Err("native registry history absent".into());
        }
        if complete {
            self.tick = self.tick.checked_add(1).ok_or("tick counter exhausted")?;
        }
        Ok(complete)
    }
    pub fn interrupt(&mut self, id: ThreadId) -> Result<(), String> {
        let t = self.threads.get_mut(&id).ok_or("unknown thread")?;
        t.interrupted = true;
        if matches!(t.status, Status::Sleeping { .. }) {
            t.status = Status::Ready;
        }
        Ok(())
    }
    pub fn register_object_thread(&mut self, object: ObjectId, id: ThreadId) -> Result<(), String> {
        let t = self.threads.get(&id).ok_or("unknown object thread")?;
        if t.check
            || t.stack.first().is_none_or(|f| f.caller != object)
            || self.object_threads.contains_key(&object)
        {
            return Err("invalid or duplicate object thread registration".into());
        }
        self.object_threads.insert(object, id);
        Ok(())
    }
    pub fn finish_animation(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: u64,
        outcome: AnimationOutcome,
    ) -> Result<(), String> {
        let t = self
            .threads
            .get_mut(&id)
            .ok_or("unknown animation thread")?;
        let Status::AwaitingAnimation(wait) = &t.status else {
            return Err("thread is not waiting for animation".into());
        };
        if wait.id != request {
            return Err("stale animation response".into());
        }
        let ins = current(program, t)?.clone();
        let success = match outcome {
            AnimationOutcome::Completed => true,
            AnimationOutcome::Event(value) => {
                let f = t.stack.last_mut().ok_or("missing animation frame")?;
                if ins.operands[5] & 32 != 0 {
                    f.opaque_locals.remove(&usize::from(ins.operands[2]));
                    *cell_mut(
                        &mut f.locals,
                        i16::from(ins.operands[2]),
                        "animation event local",
                    )? = value;
                } else {
                    *cell_mut(&mut f.args, 0, "animation event parameter")? = value;
                }
                false
            }
        };
        self.complete_host_primitive(program, id, success, ins.on_true, ins.on_false)
    }
    /// Called by the routing host after applying the actual route's world effects.
    pub fn finish_route(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: u64,
        success: bool,
    ) -> Result<(), String> {
        let t = self.threads.get_mut(&id).ok_or("unknown thread")?;
        match &t.status {
            Status::AwaitingRoute(r) if r.id == request => (),
            _ => return Err("stale, duplicate, or mismatched route response".into()),
        }
        let ins = current(program, t)?.clone();
        self.complete_host_primitive(program, id, success, ins.on_true, ins.on_false)
    }
    pub fn finish_turn(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: u64,
        success: bool,
    ) -> Result<(), String> {
        let t = self.threads.get(&id).ok_or("unknown turn thread")?;
        if !matches!(&t.status, Status::AwaitingTurn(r) if r.id == request) {
            return Err("stale or mismatched turn response".into());
        }
        let ins = current(program, t)?.clone();
        self.complete_host_primitive(program, id, success, ins.on_true, ins.on_false)
    }
    pub fn finish_transfer(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: u64,
        success: bool,
    ) -> Result<(), String> {
        let t = self.threads.get(&id).ok_or("missing transfer thread")?;
        if !matches!(&t.status,Status::AwaitingTransfer(r) if r.id==request) {
            return Err("stale transfer response".into());
        }
        let ins = current(program, t)?.clone();
        self.complete_host_primitive(program, id, success, ins.on_true, ins.on_false)
    }
    pub fn finish_snap(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: u64,
        success: bool,
    ) -> Result<(), String> {
        let t = self.threads.get_mut(&id).ok_or("unknown thread")?;
        match &t.status {
            Status::AwaitingSnap(r) if r.route.id == request => (),
            _ => return Err("stale, duplicate, or mismatched snap response".into()),
        }
        let ins = current(program, t)?.clone();
        self.complete_host_primitive(program, id, success, ins.on_true, ins.on_false)
    }
    /// Host-serviced primitives return through the same native frame completion
    /// path as synchronous primitives, including action use counts and last result.
    fn complete_host_primitive(
        &mut self,
        program: &Program,
        id: ThreadId,
        success: bool,
        yes: u8,
        no: u8,
    ) -> Result<(), String> {
        let mut t = self.threads.remove(&id).ok_or("unknown thread")?;
        t.status = Status::Ready;
        t.last_result = success;
        t.stack
            .last_mut()
            .ok_or("missing host primitive frame")?
            .continuation = 0;
        if let Err(e) =
            self.branch_in_thread(program, id, &mut t, if success { yes } else { no })
        {
            fault(&mut t, e);
        }
        if let Err(e) = self.finish_action_frames(&mut t) {
            fault(&mut t, e);
        }
        // A host result can return the root Main or the synchronous Init just
        // like an instruction executed inside run_slice. Publish its lifecycle
        // transition before a budget boundary can capture the resumed thread.
        if let Err(e) = self.continue_lifecycle(program, id, &mut t) {
            fault(&mut t, e);
        }
        self.threads.insert(id, t);
        Ok(())
    }
    fn run_local_slice(
        &mut self,
        program: &Program,
        id: ThreadId,
        budget: usize,
    ) -> Result<Run, String> {
        self.reject_native_suspension_run(id)?;
        let mut t = self.threads.remove(&id).ok_or("unknown thread")?;
        let scheduled_object = t
            .stack
            .first()
            .map(|f| f.caller)
            .filter(|object| self.object_threads.get(object) == Some(&id));
        // Synchronous checks/callbacks bypass the lot scheduler. Only the
        // registered object's ordinary execution consumes its scheduler visit.
        let explicit_visit = scheduled_object.is_some_and(|object| {
            self.scheduled_object_visit
                .as_ref()
                .is_some_and(|v| v.owns(object, id, self.tick))
        });
        if !explicit_visit
            && budget > 0
            && !t.check
            && matches!(t.status, Status::Ready | Status::Sleeping { .. })
        {
            if let Some(idle) = scheduled_object
                .and_then(|object| self.world.objects.get_mut(&object))
                .and_then(|o| o.idle.as_mut())
            {
                if idle.last_run != Some(self.tick) && !idle.visit(self.tick) {
                    if t.status == Status::Ready {
                        t.status = Status::Sleeping {
                            wake_tick: self.tick.saturating_add(idle.countdown as u64 + 1),
                        };
                    }
                    self.threads.insert(id, t);
                    return Ok(Run {
                        stop: Stop::Sleeping,
                        trace: Vec::new(),
                    });
                }
            }
        }
        if let Status::Sleeping { wake_tick } = t.status
            && self.tick >= wake_tick
        {
            t.status = Status::Ready;
        }
        let mut trace = Vec::new();
        for _ in 0..budget {
            if t.status != Status::Ready {
                break;
            }
            match self.next_action_callback(program) {
                Ok(Some(_)) => {
                    self.threads.insert(id, t);
                    return Ok(Run {
                        stop: Stop::ActionCallbacks,
                        trace,
                    });
                }
                Err(e) => {
                    fault(&mut t, e);
                    break;
                }
                Ok(None) => (),
            }
            let ins = match current(program, &t) {
                Ok(ins) => ins.clone(),
                Err(e) => {
                    fault(&mut t, e);
                    break;
                }
            };
            let frame = t.stack.last().expect("validated current frame");
            let mut entry = Trace {
                tick: self.tick,
                thread: id,
                depth: t.stack.len(),
                location: frame.location.clone(),
                opcode: ins.opcode,
                operands: ins.operands,
                effect: String::new(),
            };
            let callback_depth = t.callback.as_ref().map(|b| b.base_depth);
            let in_service = t.service.as_ref().is_some_and(|s| s.result.is_none());
            let previous_thread = self.terminal_observer.executing_thread.replace(id);
            let effect = self.execute(program, id, &mut t, &ins);
            self.terminal_observer.executing_thread = previous_thread;
            entry.effect = match effect {
                Ok(effect) => effect,
                Err(e) => {
                    fault(&mut t, e.clone());
                    format!("FAULT: {e}")
                }
            };
            if let Err(e) = self.finish_action_frames(&mut t) {
                fault(&mut t, e);
            }
            trace.push(entry);
            if callback_depth.is_some()
                && t.callback.as_ref().map(|b| b.base_depth) != callback_depth
            {
                break;
            }
            if self.named_calls.last().is_some_and(|c| {
                c.requester == id && c.depth == t.stack.len() && c.result.is_none()
            }) {
                break;
            }
            if in_service && t.service.as_ref().is_some_and(|s| s.result.is_some()) {
                break;
            }
        }
        if let Err(e) = self.continue_lifecycle(program, id, &mut t) {
            fault(&mut t, e);
        }
        let stop = match t.status {
            Status::Ready => Stop::Budget,
            Status::Returned(v) => Stop::Returned(v),
            Status::Sleeping { .. } => Stop::Sleeping,
            // A real callback may return to its native-suspended parent.
            // Direct entry was rejected before execution; no timer is inferred.
            Status::NativeSuspended(_) | Status::RegistrySuspended(_) | Status::NativeRegistryReentry(_) => Stop::Sleeping,
            Status::AwaitingRoute(_) => Stop::AwaitingRoute,
            Status::AwaitingSnap(_) => Stop::AwaitingSnap,
            Status::AwaitingAnimation(_) => Stop::AwaitingAnimation,
            Status::AwaitingAccessory { .. } => Stop::AwaitingAccessory,
            Status::AwaitingTurn(_) => Stop::AwaitingTurn,
            Status::AwaitingReach(_) => Stop::AwaitingReach,
            Status::AwaitingCreation(_) => Stop::AwaitingCreation,
            Status::AwaitingImmediate(_) => Stop::AwaitingImmediate,
            Status::AwaitingTransfer(_) => Stop::AwaitingTransfer,
            Status::AwaitingPlacement(_) => Stop::AwaitingPlacement,
            Status::AwaitingRemoval(_) => Stop::AwaitingRemoval,
            Status::AwaitingBurn(_) => Stop::AwaitingBurn,
            Status::AwaitingDialog(_) => Stop::AwaitingDialog,
            Status::AwaitingUiEffect(_) => Stop::AwaitingUiEffect,
            Status::Faulted(_) => Stop::Faulted,
        };
        self.threads.insert(id, t);
        Ok(Run { stop, trace })
    }
    fn execute(
        &mut self,
        program: &Program,
        thread_id: ThreadId,
        t: &mut Thread,
        ins: &Instruction,
    ) -> Result<String, String> {
        if ins.opcode >= 256 {
            if ins.opcode & 0x8000 != 0 {
                return Err("flagged native instruction dispatch is unsupported".into());
            }
            return call(
                program,
                t,
                ins,
                &self.world.objects[&t.stack[0].caller].temps,
            );
        }
        if ins.opcode == 0x24 {
            return self.dialog_primitive(program, t, ins);
        }
        if ins.opcode == 0x22 {
            return self.ui_effect_primitive(program, t, ins);
        }
        if ins.opcode == 0x23 {
            return self.special_effect(program, t, ins);
        }
        if ins.opcode == 0x0e {
            return self.find_best_function(program, t, ins);
        }
        if ins.opcode == 0x14 {
            return self.run_functional_tree(program, t, ins);
        }
        if ins.opcode == 0x1c {
            return self.run_named_tree(program, thread_id, t, ins);
        }
        if matches!(ins.opcode, 4 | 5 | 43) {
            let f = t.stack.last().ok_or("missing transfer frame")?;
            t.status = Status::AwaitingTransfer(transfer::Request {
                id: self.next_request,
                origin: f.location.clone(),
                caller: f.caller,
                target: f.stack_object,
                grab: ins.opcode == 4,
                onto: if ins.opcode == 43 {
                    Some(transfer::onto_slots(&ins.operands, &f.args)?)
                } else {
                    None
                },
            });
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or("request ID overflow")?;
            return Ok("awaiting Grab/Drop placement".into());
        }
        if ins.opcode == 9 {
            let f = t.stack.last().ok_or("missing Burn frame")?;
            t.status = Status::AwaitingBurn(burn::Request {
                id: self.next_request,
                origin: f.location.clone(),
                caller: f.caller,
                target: f.stack_object,
                operands: ins.operands,
                started: false,
            });
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or("request ID overflow")?;
            return Ok("awaiting synchronous Burn services".into());
        }
        if ins.opcode == 0x12 {
            let f = t.stack.last().ok_or("missing removal frame")?;
            let target = removal::selection(word(&ins.operands, 0), f.caller, f.stack_object)?;
            if !self.world.objects.contains_key(&target) {
                self.branch(program, t, ins.on_false)?;
                return Ok("Remove Object missing target; branch false".into());
            }
            t.status = Status::AwaitingRemoval(removal::Request {
                id: self.next_request,
                origin: f.location.clone(),
                caller: f.caller,
                target,
                flags: ins.operands[2],
            });
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or("request ID overflow")?;
            return Ok("awaiting deferred object removal".into());
        }
        if ins.opcode == 0x10 {
            if t.check {
                return Err("Find Location in a synchronous check is unsupported".into());
            }
            t.status = Status::AwaitingPlacement(self.describe_find_location(
                t,
                &ins.operands,
                self.next_request,
            )?);
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or("request ID overflow")?;
            return Ok("awaiting Find Location search and placement".into());
        }
        if ins.opcode == 0x2a {
            let f = t.stack.last().ok_or("missing creation frame")?;
            t.status = Status::AwaitingCreation(create::Request {
                id: self.next_request,
                origin: f.location.clone(),
                caller: f.caller,
                target: f.stack_object,
                operands: ins.operands,
                created: None,
                placement: None,
            });
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or("request ID overflow")?;
            return Ok("awaiting original object factory and placement".into());
        }
        if ins.opcode == 0x1e {
            return self.gosub_found_action(program, thread_id, t, ins.on_true, ins.on_false);
        }
        if ins.opcode == 3 {
            return self.find_best_interaction(program, t, ins.on_true, ins.on_false);
        }
        if ins.opcode == 0x16 && word(&ins.operands, 0) != 0 {
            return self.request_turn(program, t, ins);
        }
        let (result, effect) = match ins.opcode {
            0x0a => {
                let caller = t.stack.last().ok_or("missing UI-owner frame")?.caller;
                let result = self.world.ui_owner.primitive(caller, ins.operands[0])?;
                self.world.apply_dialog_closes()?;
                (
                    result,
                    format!("UI-event ownership mode {}: {result}", ins.operands[0]),
                )
            }
            0x1a => self.relationship(program, t, &ins.operands)?,
            0x21 => self.find_worst_motives(program, t, &ins.operands)?,
            0x06 => {
                if ins.operands[1] != 1 && (self.native_accessory_host || ins.operands[2] & 1 == 0)
                {
                    let name = self.accessory_label(program, t, &ins.operands)?;
                    if self.native_accessory_host || !program.accessories.contains_key(&name) {
                        let request = if self.native_accessory_host {
                            let id = self.next_request;
                            self.next_request = id.checked_add(1).ok_or("request ID overflow")?;
                            id
                        } else {
                            0
                        };
                        let f = t.stack.last().unwrap();
                        t.status = Status::AwaitingAccessory {
                            request,
                            origin: f.location.clone(),
                            caller: f.caller,
                            name: name.clone(),
                        };
                        return Ok(format!(
                            "await accessory asset {name:?}; instruction not executed"
                        ));
                    }
                }
                self.change_accessory(program, t, &ins.operands)?
            }

            0x01 => self.generic_person_call(t, &ins.operands)?,
            0x19 => self.change_budget(program, t, &ins.operands)?,
            0x32 => self.change_action_string(program, thread_id, t, &ins.operands)?,
            0x16 => self.look_towards(t, &ins.operands)?,
            0x1d => self.set_motive_changes(program, t, &ins.operands)?,
            0x29 => self.set_headline(program, t, &ins.operands)?,
            0x1f => self.set_to_next(program, t, &ins.operands)?,
            0x25 => self.test_interacting(program, t, &ins.operands)?,
            0x33 => self.inventory(t, &ins.operands)?,
            0x20 => {
                let object = self.read(program, t, ins.operands[6], word(&ins.operands, 4))?;
                let guid = u32::from_le_bytes(ins.operands[..4].try_into().unwrap());
                (
                    multitile::is_type(&self.world, program, object, guid)?,
                    format!("object #{object} type matches GUID {guid:#010x}"),
                )
            }
            0x17 => {
                let f = t.stack.last().ok_or("missing sound frame")?;
                let event = u16::from_le_bytes(ins.operands[..2].try_into().unwrap());
                let name = program
                    .modules
                    .get(&f.code_owner)
                    .and_then(|m| m.sounds.get(&event))
                    .or_else(|| {
                        program
                            .modules
                            .get(&program.globals)
                            .and_then(|m| m.sounds.get(&event))
                    })
                    .cloned();
                let object = if ins.operands[4] != 255 && ins.operands[4] & 2 != 0 {
                    f.stack_object
                } else {
                    f.caller
                };
                if self.sounds.len() >= 4096 {
                    return Err(
                        "sound presentation queue is full; drain it before continuing".into(),
                    );
                }
                if let Some(name) = &name {
                    self.sounds.push(SoundEvent {
                        tick: self.tick,
                        object,
                        stop: false,
                        name: name.clone(),
                        flags: ins.operands[4],
                        sample_rate: u16::from_le_bytes(ins.operands[2..4].try_into().unwrap()),
                        volume: ins.operands[5],
                    });
                }
                (
                    true,
                    format!(
                        "sound {:?} on #{object}; no audio device in headless VM",
                        name
                    ),
                )
            }
            0x30 => {
                let f = t.stack.last().ok_or("missing sound frame")?;
                let object = if i16::from_le_bytes(ins.operands[..2].try_into().unwrap()) == 0 {
                    f.caller
                } else {
                    f.stack_object
                };
                if self.sounds.len() >= 4096 {
                    return Err(
                        "sound presentation queue is full; drain it before continuing".into(),
                    );
                }
                self.sounds.push(SoundEvent {
                    tick: self.tick,
                    object,
                    stop: true,
                    name: String::new(),
                    flags: 0,
                    sample_rate: 0,
                    volume: 0,
                });
                (true, format!("queued stop all sounds on #{object}"))
            }
            7 => {
                let f = t.stack.last().ok_or("missing Refresh frame")?;
                self.refresh_object(
                    f.caller,
                    f.stack_object,
                    word(&ins.operands, 0),
                    word(&ins.operands, 2),
                )?;
                (
                    true,
                    format!("Refresh mode {} applied", word(&ins.operands, 2)),
                )
            }
            0x31 => {
                let object = t.stack.last().ok_or("missing Notify frame")?.stack_object;
                self.notify_idle(program, thread_id, t, object)?;
                (
                    true,
                    format!("notified object #{object} at its current idle instruction"),
                )
            }
            0 => {
                if t.check && t.callback.is_none() {
                    return Err("sleep is unsupported inside an eligibility check".into());
                }
                if self.sleep(t, word(&ins.operands, 0))? {
                    (true, "sleep completed".into())
                } else {
                    if t.callback.is_some() {
                        return Err("synchronous callback yielded (native error 0x44f)".into());
                    }
                    return Ok("sleep yielded".into());
                }
            }
            24 => self.legacy_relationship(program, t, &ins.operands)?,
            2 => self.expression(program, t, &ins.operands)?,
            8 => self.random_primitive(program, t, &ins.operands)?,
            11 | 12 => self.spatial_query(program, t, ins.opcode, &ins.operands)?,
            13 => self.push_interaction(program, t, &ins.operands)?,
            17 => {
                return self.idle_for_input(program, thread_id, t, &ins.operands, ins.on_true);
            }
            0x1b | 0x2d => {
                if t.check {
                    (false, "routing request in eligibility check → false".into())
                } else if ins.opcode == 0x1b
                    && routing::relative_position(&ins.operands, 0)
                        == routing::Preparation::InvalidFacing
                {
                    (
                        false,
                        "relative route invalid facing; original diagnostic branch → false".into(),
                    )
                } else {
                    return self.request_route(program, t, &ins.operands);
                }
            }
            0x2e => {
                let id = self.next_request;
                let request = self.describe_snap(program, t, &ins.operands, id)?;
                self.next_request = id.checked_add(1).ok_or("request ID exhausted")?;
                t.status = Status::AwaitingSnap(request);
                return Ok(format!("await snap service, request {id}"));
            }
            0x2f => {
                if t.check {
                    return Err("Reach in a synchronous check is unsupported".into());
                }
                let f = t.stack.last().ok_or("missing Reach frame")?;
                if self.world.objects[&f.caller].person.is_none() {
                    return Err("Reach requires a person (native error 0x05)".into());
                }
                let id = self.next_request;
                self.next_request = id.checked_add(1).ok_or("request ID exhausted")?;
                t.status = Status::AwaitingReach(reach::Request {
                    id,
                    origin: f.location.clone(),
                    caller: f.caller,
                    target: f.stack_object,
                    operands: ins.operands,
                });
                return Ok(format!("await Reach service, request {id}"));
            }
            0x2c => {
                if t.check {
                    return Err("animation in an eligibility check is unsupported".into());
                }
                let f = t.stack.last().ok_or("no animation frame")?;
                let id = self.next_request;
                self.next_request = id.checked_add(1).ok_or("request ID exhausted")?;
                t.status = Status::AwaitingAnimation(AnimationRequest {
                    id,
                    origin: f.location.clone(),
                    caller: f.caller,
                    operands: ins.operands,
                });
                return Ok("await animation service response".into());
            }
            opcode => {
                return Err(format!(
                    "unsupported primitive {opcode:#06x} ({}) with operands {:02X?}",
                    primitive_name(opcode),
                    ins.operands
                ));
            }
        };
        if matches!(t.status, Status::AwaitingImmediate(_)) {
            return Ok(effect);
        }
        t.stack
            .last_mut()
            .ok_or("primitive lost its frame")?
            .continuation = 0;
        t.last_result = result;
        self.branch(program, t, if result { ins.on_true } else { ins.on_false })?;
        Ok(format!("{effect}; branch {result}"))
    }
    fn request_route(
        &mut self,
        program: &Program,
        t: &mut Thread,
        bytes: &[u8; 8],
    ) -> Result<String, String> {
        let id = self.next_request;
        let request = self.describe_route(program, t, bytes, id)?;
        self.next_request = id.checked_add(1).ok_or("request ID exhausted")?;
        let effect = format!(
            "await routing service, request {id}: {:?}; no movement has occurred",
            request.destination
        );
        t.status = Status::AwaitingRoute(request);
        Ok(effect)
    }
    fn describe_route(
        &self,
        program: &Program,
        t: &Thread,
        bytes: &[u8; 8],
        id: u64,
    ) -> Result<RouteRequest, String> {
        let frame = t.stack.last().ok_or("no route frame")?;
        if current(program, t)?.opcode == 0x1b {
            let routing::Preparation::Start(descriptor) = routing::relative_position(bytes, 0)
            else {
                return Err("relative route cannot start with invalid facing".into());
            };
            if !self.world.objects.contains_key(&frame.stack_object) {
                return Err("relative route target is missing".into());
            }
            let no_failure_trees = descriptor.flags & 0x4000 == 0;
            return Ok(RouteRequest {
                id,
                origin: frame.location.clone(),
                caller: frame.caller,
                target: frame.stack_object,
                destination: routing::Destination::Relative(descriptor),
                no_failure_trees,
            });
        }
        let data = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        let scope = u16::from_le_bytes([bytes[2], bytes[3]]);
        if bytes[4] & !1 != 0 {
            return Err(format!("unsupported routing flags {:#04x}", bytes[4]));
        }
        let target = self
            .world
            .objects
            .get(&frame.stack_object)
            .ok_or("routing target is missing")?;
        let (source, chunk, index) = match scope {
            0 | 1 => {
                let def = target
                    .definition
                    .as_ref()
                    .ok_or("routing target has no definition")?;
                let index = if scope == 0 {
                    usize::try_from(cell(&frame.args, data as i16, "routing parameter")?)
                        .map_err(|_| "negative routing index")?
                } else {
                    data
                };
                (&def.source, def.slots, index)
            }
            2 => (&program.globals, 100, data),
            _ => return Err(format!("unsupported slot scope {scope}")),
        };
        if scope != 2
            && let Some(slots) = &target.routing_slots
        {
            let slot = slots
                .get(index)
                .ok_or("missing object-owned routing slot (native error 0x25)")?
                .slot();
            return Ok(RouteRequest {
                id,
                origin: frame.location.clone(),
                caller: frame.caller,
                target: frame.stack_object,
                destination: routing::Destination::Slot {
                    source: source.clone(),
                    chunk,
                    index,
                    version: 10,
                    slot,
                },
                no_failure_trees: bytes[4] & 1 != 0,
            });
        }
        let slots = program
            .modules
            .get(source)
            .and_then(|m| m.slots.get(&chunk))
            .ok_or_else(|| format!("missing SLOT #{chunk} in {source}"))?;
        let slot = slots.routing(index).ok_or_else(|| {
            format!("missing routing slot index {index} in {source} SLOT #{chunk}")
        })?;
        Ok(RouteRequest {
            id,
            origin: frame.location.clone(),
            caller: frame.caller,
            target: frame.stack_object,
            destination: routing::Destination::Slot {
                source: source.clone(),
                chunk,
                index,
                version: slots.version,
                slot: slot.clone(),
            },
            no_failure_trees: bytes[4] & 1 != 0,
        })
    }
    fn describe_snap(
        &self,
        program: &Program,
        t: &Thread,
        bytes: &[u8; 8],
        id: u64,
    ) -> Result<SnapRequest, String> {
        if t.check {
            return Err("snap in an eligibility check is unsupported".into());
        }
        let mut route_bytes = *bytes;
        let scope = u16::from_le_bytes([bytes[2], bytes[3]]);
        if bytes[4] & !6 != 0 {
            return Err(format!("unsupported snap flags {:#04x}", bytes[4]));
        }
        if matches!(scope, 1 | 2) {
            let frame = t.stack.last().ok_or("no Snap frame")?;
            if !self.world.objects.contains_key(&frame.stack_object) {
                return Err("Snap target missing (native error 0x15)".into());
            }
            return Ok(SnapRequest {
                route: RouteRequest {
                    id,
                    origin: frame.location.clone(),
                    caller: frame.caller,
                    target: frame.stack_object,
                    no_failure_trees: false,
                    destination: if scope == 1 {
                        routing::Destination::FirstSlot
                    } else {
                        routing::Destination::Front
                    },
                },
                flags: bytes[4],
            });
        }
        let route_scope: u16 = match scope {
            0 => 0,
            3 => 1,
            4 => 2,
            _ => return Err(format!("unsupported snap scope {scope}")),
        };
        if bytes[4] & !6 != 0 {
            return Err(format!("unsupported snap flags {:#04x}", bytes[4]));
        }
        route_bytes[2..4].copy_from_slice(&route_scope.to_le_bytes());
        route_bytes[4] = 0;
        Ok(SnapRequest {
            route: self.describe_route(program, t, &route_bytes, id)?,
            flags: bytes[4],
        })
    }
}
pub fn primitive_name(opcode: u16) -> &'static str {
    match opcode {
        0 => "sleep",
        2 => "expression",
        6 => "change suit/accessory",
        8 => "random number",
        10 => "UI-event ownership",
        15 => "breakpoint",
        0x17 => "play sound",
        0x18 => "old relationship",
        0x2f => "reach",
        0x0e => "find best object for function",
        0x1b => "go to relative position",
        0x20 => "test object type",
        0x21 => "find five worst motives",
        0x25 => "test Sim interacting with",
        0x2c => "animate Sim",
        0x2d => "go to routing slot",
        0x2e => "snap",
        0x31 => "notify out of idle",
        _ => "unimplemented",
    }
}
pub(crate) fn dimensions(routine: &Routine) -> Result<(usize, usize), String> {
    if routine.body.instructions.is_empty() {
        return Err("BHAV has no instructions".into());
    }
    let (args, locals) = match routine.body.format {
        0x8000 => (4, 0),
        0x8001 => (routine.body.args.min(4), (routine.body.locals as u8).min(4)),
        0x8002 => (routine.body.args, routine.body.locals as u8),
        _ => return Err("unsupported native BHAV version".into()),
    };
    let (args, locals) = (usize::from(args), usize::from(locals));
    Ok((args, locals))
}
pub(crate) fn current<'a>(program: &'a Program, t: &Thread) -> Result<&'a Instruction, String> {
    let frame = t.stack.last().ok_or("empty execution stack")?;
    let (source, routine) = program.resolve(&frame.code_owner, frame.location.bhav)?;
    if source != frame.location.source {
        return Err("routine source/context mismatch".into());
    }
    if frame.location.instruction > 0xfa {
        return Err("invalid native instruction index (native error 0x44c)".into());
    }
    routine
        .body
        .instructions
        .get(frame.location.instruction)
        .ok_or_else(|| "instruction pointer outside routine".into())
}
fn call(
    program: &Program,
    t: &mut Thread,
    ins: &Instruction,
    temps: &[i16; 8],
) -> Result<String, String> {
    if t.stack.len() >= MAX_STACK {
        return Err("call stack limit reached".into());
    }
    let parent = t.stack.last().ok_or("no caller frame")?;
    let (source, routine) = program.resolve(&parent.code_owner, ins.opcode)?;
    let (argc, locals) = dimensions(routine)?;
    let args = if ins.operands == [255; 8] {
        temps
            .get(..argc)
            .ok_or("native argument read beyond eight temporaries is unsupported")?
            .to_vec()
    } else {
        // Native copies a contiguous argc-word span beginning at the instruction operands.
        let (_, parent_routine) = program.resolve(&parent.code_owner, parent.location.bhav)?;
        let mut bytes = ins.operands.to_vec();
        for next in &parent_routine.body.instructions[parent.location.instruction + 1..] {
            bytes.extend_from_slice(&next.opcode.to_le_bytes());
            bytes.extend_from_slice(&[next.on_true, next.on_false]);
            bytes.extend_from_slice(&next.operands);
            if bytes.len() >= argc * 2 {
                break;
            }
        }
        bytes
            .get(..argc * 2)
            .ok_or("native call argument read past BHAV resource is unsupported")?
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    };
    let pc = entry_pc(
        routine,
        if ins.operands == [255; 8] {
            temps[0]
        } else {
            word(&ins.operands, 0)
        },
    )?;
    let mut frame = Frame {
        continuation: 0,
        action_target: None,
        action_icon: false,
        code_owner: parent.code_owner.clone(),
        location: Location {
            source: source.into(),
            bhav: ins.opcode,
            instruction: pc,
        },
        caller: parent.caller,
        callee: parent.callee,
        stack_object: parent.stack_object,
        args,
        locals: vec![0; locals],
        opaque_locals: Default::default(),
        storage_offset: parent.storage_offset + frame_words(parent.args.len(), parent.locals.len()),
    };
    let context = retained_frames::inherited_context(t, parent)?;
    let end = frame.storage_offset + frame_words(argc, locals);
    retained_frames::ensure_capacity(t, end)?;
    save_frame_storage(t)?;
    t.frame_memory
        .resize(t.frame_memory.len().max(end), Some(0));
    for (i, value) in frame.locals.iter_mut().enumerate() {
        match t.frame_memory[frame.storage_offset + 8 + argc + i] {
            Some(v) => *value = v,
            None => {
                frame.opaque_locals.insert(i);
            }
        }
    }
    retained_frames::finish_entry(t, frame, context)?;
    Ok(format!(
        "call {source} BHAV {:#06x} {}",
        ins.opcode, routine.name
    ))
}

impl Vm {
    pub(crate) fn branch_in_thread(
        &mut self,
        program: &Program,
        thread: ThreadId,
        t: &mut Thread,
        target: u8,
    ) -> Result<(), String> {
        let previous_thread = self.terminal_observer.executing_thread.replace(thread);
        let result = self.branch(program, t, target);
        self.terminal_observer.executing_thread = previous_thread;
        result
    }
    pub(crate) fn branch(&mut self, program: &Program, t: &mut Thread, target: u8) -> Result<(), String> {
        branch(program, t, target, &mut self.terminal_observer)
    }
}
fn branch(
    program: &Program,
    t: &mut Thread,
    mut target: u8,
    observer: &mut observations::ActionObserver,
) -> Result<(), String> {
    loop {
        match target {
            0xfd => return Err("explicit ERROR branch (0xFD)".into()),
            0xfe | 0xff => {
                let value = target == 0xfe;
                t.stack
                    .last_mut()
                    .ok_or("return from empty stack")?
                    .location
                    .instruction = usize::from(target);
                save_frame_storage(t)?;
                let popped = t.stack.pop().ok_or("return from empty stack")?;
                retained_frames::remove_frame(t, popped.storage_offset);
                observer.returned(
                    observer.executing_thread,
                    &popped,
                    t.stack.len() + 1,
                    value,
                    !t.stack.iter().any(|frame| frame.action_target.is_some()),
                );
                if let Some(target) = popped.action_target {
                    t.completed_actions.push((
                        popped.caller,
                        target,
                        t.last_result,
                        popped.action_icon,
                        t.stack.len() + 1,
                    ));
                }
                t.last_result = value;
                if t.callback
                    .as_ref()
                    .is_some_and(|b| b.base_depth == t.stack.len())
                {
                    let mut b = t.callback.take().unwrap();
                    t.callback = b.parent.take().map(|b| *b);
                    t.check = b.resume_check;
                    t.status = b.resume_status;
                    t.callback_result = Some(value);
                    return Ok(());
                }
                if let Some(service) = &mut t.service
                    && service.base_depth == t.stack.len()
                {
                    service.result = Some(value);
                    t.status = service.resume_status.clone();
                    return Ok(());
                }
                if t.stack.is_empty() {
                    t.status = Status::Returned(value);
                    return Ok(());
                }
                if t.stack.last().unwrap().continuation != 0 {
                    return Ok(());
                }
                let ins = current(program, t)?;
                target = if value { ins.on_true } else { ins.on_false };
            }
            _ => {
                if target > 0xfa {
                    return Err("invalid native branch (native error 0x44c)".into());
                }
                let frame = t.stack.last().ok_or("branch without frame")?;
                let (_, routine) = program.resolve(&frame.code_owner, frame.location.bhav)?;
                if usize::from(target) >= routine.body.instructions.len() {
                    return Err(format!("invalid branch target {target}"));
                }
                t.stack.last_mut().unwrap().location.instruction = target as usize;
                return Ok(());
            }
        }
    }
}
fn fault(t: &mut Thread, message: String) {
    t.status = Status::Faulted(Fault {
        message,
        stack: t.stack.iter().map(|f| f.location.clone()).collect(),
    });
}
fn word(bytes: &[u8; 8], offset: usize) -> i16 {
    i16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn cell(values: &[i16], index: i16, name: &str) -> Result<i16, String> {
    values
        .get(index as usize)
        .copied()
        .ok_or_else(|| format!("{name} index {index} out of bounds"))
}
fn cell_mut<'a>(values: &'a mut [i16], index: i16, name: &str) -> Result<&'a mut i16, String> {
    values
        .get_mut(index as usize)
        .ok_or_else(|| format!("{name} index {index} out of bounds"))
}

fn frame_words(args: usize, locals: usize) -> usize {
    8 + ((args + locals + 3) & !3)
}
fn entry_pc(routine: &Routine, entry: i16) -> Result<usize, String> {
    if routine.body.kind != 4 {
        return Ok(0);
    }
    if entry < 0 {
        return Err("negative indexed BHAV entry".into());
    }
    Ok((entry as usize).min(routine.body.instructions.len() - 1))
}
fn save_frame_storage(t: &mut Thread) -> Result<(), String> {
    // Validate every required metadata/capacity owner before the first write.
    for f in &t.stack {
        retained_frames::ensure_capacity(
            t,
            f.storage_offset + frame_words(f.args.len(), f.locals.len()),
        )?;
        retained_frames::context_word(t, f)?;
    }
    for f in &t.stack {
        let start = f.storage_offset;
        let end = start + frame_words(f.args.len(), f.locals.len());
        t.frame_memory
            .resize(t.frame_memory.len().max(end), Some(0));
        let mut header = frame_header(f);
        if let Some(context) = retained_frames::context_word(t, f)? {
            header[6] = Some(context as i16);
            header[7] = Some((context >> 16) as i16);
        }
        t.frame_memory[start..start + 8].copy_from_slice(&header);
        for (i, v) in f.args.iter().chain(&f.locals).enumerate() {
            t.frame_memory[start + 8 + i] =
                if i >= f.args.len() && f.opaque_locals.contains(&(i - f.args.len())) {
                    None
                } else {
                    Some(*v)
                };
        }
    }
    Ok(())
}

fn frame_header(f: &Frame) -> [Option<i16>; 8] {
    [
        Some(f.location.bhav as i16),
        Some(f.location.instruction as i16),
        Some(f.stack_object),
        Some((f.locals.len() | (f.args.len() << 8)) as i16),
        Some(f.continuation as i16),
        Some((f.continuation >> 16) as i16),
        None,
        None,
    ]
}

/// Read the native arena with live frames overlaid, without flushing/mutating
/// the saved arena. A raw native pointer word has no portable numeric value.
fn raw_frame_word(t: &Thread, address: i64) -> Result<i16, String> {
    let address = usize::try_from(address).map_err(|_| "raw frame read before arena")?;
    for f in t.stack.iter().rev() {
        let Some(offset) = address.checked_sub(f.storage_offset) else {
            continue;
        };
        let value = if offset < 8 {
            frame_header(f)[offset]
        } else if offset < 8 + f.args.len() {
            Some(f.args[offset - 8])
        } else if offset < 8 + f.args.len() + f.locals.len() {
            let local = offset - 8 - f.args.len();
            (!f.opaque_locals.contains(&local)).then_some(f.locals[local])
        } else {
            continue;
        };
        return value.ok_or_else(|| "raw frame read of opaque native pointer word".into());
    }
    t.frame_memory
        .get(address)
        .copied()
        .ok_or("raw frame read outside retained arena")?
        .ok_or_else(|| "raw frame read of opaque native pointer word".into())
}
