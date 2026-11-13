//! Explicit Kernel32 and opt-in OLE32/WINMM/ADVAPI32/private Gdi32 services.
//! None executes a PE
//! entry nor admits a driver frame, patches an import, or loads a native DLL.
//!
//! Parent integration must retain this registry beside its issuing `Memory`,
//! retain each `ProcedureLease` in a typed BootstrapServices capability, and
//! obtain logical words exclusively from that service's capability allocator.
//! Weak module/procedure observations need distinct, non-owning capability
//! variants. There is deliberately no raw-word adoption or callback API here.
//!
//! Required parent-only Memory additions:
//! `Kind::ProgramGlobal` and `allocate_program_global(bytes: u32) ->
//! Result<Allocated, ProgramAllocationError>`, whose error variants are
//! `Refused(Stop)` and `OutOfMemory`. Success must issue a fresh, nonzero,
//! u32-width, >=8-byte-aligned ProgramGlobal allocation of exactly `bytes`
//! zero bytes, with its complete initialized image in `Allocated::Bytes`.
//! `Ok(Allocated::Null)` is a hook contract violation, not an OOM observation.
//!
//! ExitProcess supports only an explicitly configured single guest thread with
//! no guest DLL detach callbacks. Its success is a terminal transfer, with no
//! integer return. Parent must consume it with `finish` and propagate termination
//! out of the guest program; mapping it to a normal return is not integration.
//! This does not clean up an outer Application, host process, threads or DLLs.
//! Dropping a registry invalidates all access but cannot free borrowed Memory;
//! parent must finish termination before dropping/replacing a live program.

use crate::native_bootstrap::memory::{Identity, Kind, Memory, ProgramAllocationError};
use super::{program_ole32, program_raster, program_raster_context, program_registry, program_registry_store, program_winmm};
use sims_sim::saved_matrix::{Allocated, MAX_ALLOCATION_BYTES, Stop};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

const MAX_HISTORY: usize = 4096;
const MAX_NAME_BYTES: usize = 260;
const EXPORTS: [Export; 5] = [
    Export::GetModuleHandleA,
    Export::GetProcAddress,
    Export::GlobalAlloc,
    Export::GlobalFree,
    Export::ExitProcess,
];
const OLE32_EXPORTS: [Export; 1] = [Export::CoUninitialize];
const WINMM_EXPORTS: [Export; 1] = [Export::TimeGetTime];
const ADVAPI32_EXPORTS: [Export; 2] = [Export::RegCreateKeyA, Export::RegCloseKey];
const GDI32_EXPORTS: [Export; 1] = [Export::CreateCompatibleDC];

/// The five sealed implementation tables. A kind selects a table, never a
/// loaded instance or authority; exact instance observations use ModuleIdentity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    Kernel32,
    Ole32,
    Winmm,
    Advapi32,
    Gdi32,
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}
fn exhausted(message: &str) -> Stop {
    Stop::Unavailable(message.into())
}

/// No Default: enabling the concrete Kernel32 Rust-port module is explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kernel32Policy {
    Disabled,
    RustPort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitPolicy {
    Disabled,
    SingleThreadNoGuestDllDetachCallbacks,
}

/// Lifetime issuance budgets, not native Windows failure conditions. No
/// unbounded event log is kept. Freed allocations and retired modules still
/// count against these histories; old identities are never recycled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub module_issuances: usize,
    pub procedure_lease_issuances: usize,
    pub weak_observation_issuances: usize,
    pub global_allocation_issuances: usize,
    pub max_live_global_bytes: usize,
    pub max_global_bytes: u32,
    pub max_name_bytes: usize,
}

impl Limits {
    fn validate(self) -> Result<(), Stop> {
        if [
            self.module_issuances,
            self.procedure_lease_issuances,
            self.weak_observation_issuances,
            self.global_allocation_issuances,
        ].iter().any(|n| *n == 0 || *n > MAX_HISTORY)
            || self.max_live_global_bytes == 0
            || self.max_live_global_bytes > MAX_ALLOCATION_BYTES
            || self.max_global_bytes == 0
            || self.max_global_bytes as usize > self.max_live_global_bytes
            || self.max_name_bytes == 0
            || self.max_name_bytes > MAX_NAME_BYTES
        {
            return Err(unsupported("program service limits exceed the bounded profile"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Configuration {
    pub kernel32: Kernel32Policy,
    pub exit: ExitPolicy,
    pub limits: Limits,
}

/// A concrete export-table entry, never a native function address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Export {
    GetModuleHandleA,
    GetProcAddress,
    GlobalAlloc,
    GlobalFree,
    ExitProcess,
    CoUninitialize,
    TimeGetTime,
    RegCreateKeyA,
    RegCloseKey,
    CreateCompatibleDC,
}
impl Export {
    pub fn name(self) -> &'static [u8] {
        match self {
            Self::GetModuleHandleA => b"GetModuleHandleA",
            Self::GetProcAddress => b"GetProcAddress",
            Self::GlobalAlloc => b"GlobalAlloc",
            Self::GlobalFree => b"GlobalFree",
            Self::ExitProcess => b"ExitProcess",
            Self::CoUninitialize => b"CoUninitialize",
            Self::TimeGetTime => b"timeGetTime",
            Self::RegCreateKeyA => b"RegCreateKeyA",
            Self::RegCloseKey => b"RegCloseKey",
            Self::CreateCompatibleDC => b"CreateCompatibleDC",
        }
    }
    pub fn module_kind(self) -> ModuleKind {
        match self {
            Self::CoUninitialize => ModuleKind::Ole32,
            Self::TimeGetTime => ModuleKind::Winmm,
            Self::RegCreateKeyA | Self::RegCloseKey => ModuleKind::Advapi32,
            Self::CreateCompatibleDC => ModuleKind::Gdi32,
            Self::GetModuleHandleA | Self::GetProcAddress | Self::GlobalAlloc
                | Self::GlobalFree | Self::ExitProcess => ModuleKind::Kernel32,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ProcedureQuery<'a> {
    Name(&'a [u8]),
    Ordinal(u16),
}

#[derive(Debug)]
struct Lifetime {
    active: AtomicBool,
}
#[derive(Debug)]
struct Procedure {
    export: Export,
}
#[derive(Debug)]
struct Module {
    kind: ModuleKind,
    active: AtomicBool,
    procedures: Box<[Arc<Procedure>]>,
}
impl Module {
    fn new(kind: ModuleKind) -> Self {
        let exports: &[Export] = match kind {
            ModuleKind::Kernel32 => &EXPORTS,
            ModuleKind::Ole32 => &OLE32_EXPORTS,
            ModuleKind::Winmm => &WINMM_EXPORTS,
            ModuleKind::Advapi32 => &ADVAPI32_EXPORTS,
            ModuleKind::Gdi32 => &GDI32_EXPORTS,
        };
        Self {
            kind,
            active: AtomicBool::new(true),
            procedures: exports.iter().map(|&export| Arc::new(Procedure { export })).collect(),
        }
    }
    fn find(&self, name: &[u8]) -> Option<&Arc<Procedure>> {
        self.procedures.iter().find(|p| p.export.name() == name)
    }
    fn contains(&self, procedure: &Arc<Procedure>) -> bool {
        self.procedures.iter().any(|p| Arc::ptr_eq(p, procedure))
    }
}

/// No Clone, serialized identity, native module word or owning upgrade API.
#[derive(Debug)]
pub struct ModuleReference {
    registry: Weak<Lifetime>,
    module: Weak<Module>,
}
/// Immutable, non-owning identity of an authenticated loaded dependency.
/// Equality compares both issuing registry and actual module allocations.
/// This cannot be upgraded, loaded, or used as a procedure capability.
#[derive(Debug, Clone)]
pub struct ModuleIdentity {
    registry: Weak<Lifetime>,
    module: Weak<Module>,
    kind: ModuleKind,
}
impl ModuleIdentity {
    pub fn kind(&self) -> ModuleKind { self.kind }
}
impl PartialEq for ModuleIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && Weak::ptr_eq(&self.registry, &other.registry)
            && Weak::ptr_eq(&self.module, &other.module)
    }
}
impl Eq for ModuleIdentity {}
/// Neither weak reference pins the module or its actual procedure table.
#[derive(Debug)]
pub struct ProcedureObservation {
    registry: Weak<Lifetime>,
    module: Weak<Module>,
    procedure: Weak<Procedure>,
}
/// Actual import ownership. Its module dependency retains its complete concrete
/// procedure entries; it cannot be reconstructed from a lookup observation.
#[derive(Debug)]
pub struct ProcedureLease {
    registry: Weak<Lifetime>,
    module: Arc<Module>,
    procedure: Arc<Procedure>,
}

/// A lookup capability may call a still-live concrete procedure without
/// acquiring import ownership. Parent must keep these two capability roles
/// distinct; neither alternative admits an integer or arbitrary callback.
#[derive(Debug, Clone, Copy)]
pub enum ProcedureAccess<'a> {
    Import(&'a ProcedureLease),
    Lookup(&'a ProcedureObservation),
}

#[derive(Debug)]
struct AllocationIdentity;
#[derive(Debug)]
pub struct GlobalAllocation {
    registry: Weak<Lifetime>,
    issuance: Arc<AllocationIdentity>,
}
struct AllocationRecord {
    issuance: Arc<AllocationIdentity>,
    pointer: u32,
    bytes: u32,
    live: bool,
}

#[derive(Debug)]
pub enum GlobalAllocReturn {
    Allocated(GlobalAllocation),
    /// Only the typed allocator's actual supported OOM reaches this variant.
    OutOfMemory,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeFailure {
    HandleMismatch,
    AlreadyReleased,
    WrongAllocationKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalFreeReturn {
    Released,
    Failed { original_handle: u32, reason: FreeFailure },
}
impl GlobalFreeReturn {
    pub fn return_word(self) -> u32 {
        match self {
            Self::Released => 0,
            Self::Failed { original_handle, .. } => original_handle,
        }
    }
}

#[derive(Debug)]
struct TerminalIdentity {
    exit_code: u32,
    released_globals: usize,
}
/// A non-Clone terminal control transfer. No ordinary successful word exists.
#[derive(Debug)]
pub struct TerminalTransfer {
    registry: Weak<Lifetime>,
    terminal: Arc<TerminalIdentity>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Termination {
    pub exit_code: u32,
    pub released_globals: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct History {
    pub modules_issued: usize,
    pub procedure_leases_issued: usize,
    pub weak_observations_issued: usize,
    pub global_allocations_issued: usize,
    pub live_global_bytes: usize,
}

/// Bounded borrowed evidence from the registry's one retained Clock. These
/// observations cannot mutate/reset it, adopt reports as samples, or sample I/O.
/// They remain readable with the issuing Memory after terminal program control.
/// ```compile_fail
/// use sims_content::native_bootstrap::services::program_modules::TimeGetTimeObservation;
/// fn sample(observation: &mut TimeGetTimeObservation<'_>) { observation.sample(); }
/// ```
/// ```compile_fail
/// use sims_content::native_bootstrap::services::program_winmm::{Report, Sample};
/// fn adopt(report: Report) -> Sample { Sample::from(report) }
/// ```
/// ```compile_fail
/// use sims_content::native_bootstrap::services::program_winmm::Clock;
/// fn copy(clock: &Clock) -> Clock { clock.clone() }
/// ```
pub struct TimeGetTimeObservation<'a> {
    clock: &'a program_winmm::Clock,
}
impl TimeGetTimeObservation<'_> {
    pub fn policy(&self) -> program_winmm::Policy { self.clock.policy() }
    pub fn last_sample(&self) -> Option<&program_winmm::Sample> { self.clock.last_sample() }
    pub fn latest_report(&self) -> Option<&program_winmm::Report> { self.clock.latest_report() }
    pub fn first_failure_report(&self) -> Option<&program_winmm::Report> { self.clock.first_failure_report() }
}

/// Non-Clone, bound to the *actual* Memory lifetime even if another heap has
/// identical logical addresses and bytes. Mutation requires exclusive registry
/// and Memory borrows; no operation creates a guest thread or DLL callback.
pub struct Registry {
    heap: Identity,
    lifetime: Arc<Lifetime>,
    configuration: Configuration,
    ole32_policy: program_ole32::Policy,
    kernel32: Option<Arc<Module>>,
    ole32: Option<Arc<Module>>,
    winmm: Option<Arc<Module>>,
    advapi32: Option<Arc<Module>>,
    gdi32: Option<Arc<Module>>,
    // Actual device/context ownership, enabled once on this begun program.
    private_raster: Option<program_raster_context::Registry>,
    // Actual file-backed state belongs to this program, not a transient DLL slot.
    private_registry: Option<program_registry::Registry>,
    // Program lifetime, never the transient WINMM slot's lifetime. Loading,
    // lookup, import issuance and unload/reload cannot reset or sample it.
    winmm_clock: program_winmm::Clock,
    modules_issued: usize,
    leases_issued: usize,
    observations_issued: usize,
    allocations: Vec<AllocationRecord>,
    live_global_bytes: usize,
    terminal: Option<Arc<TerminalIdentity>>,
}

impl Registry {
    pub fn new(memory: &Memory, configuration: Configuration) -> Result<Self, Stop> {
        Self::new_with_ole32(memory, configuration, program_ole32::Policy::Disabled)
    }

    /// Additive opt-in; existing Configuration literals and new() remain
    /// Kernel32-only. A registry's policies can never be reconfigured.
    pub fn new_with_ole32(memory: &Memory, configuration: Configuration,
                         ole32_policy: program_ole32::Policy) -> Result<Self, Stop> {
        Self::new_with_profiles(memory, configuration, ole32_policy, program_winmm::Policy::Disabled)
    }

    /// Explicit immutable profiles for one program; construction does no I/O.
    /// Existing constructors keep WINMM disabled and Configuration unchanged.
    pub fn new_with_profiles(memory: &Memory, configuration: Configuration,
                             ole32_policy: program_ole32::Policy,
                             winmm_policy: program_winmm::Policy) -> Result<Self, Stop> {
        configuration.limits.validate()?;
        Ok(Self {
            heap: memory.identity(),
            lifetime: Arc::new(Lifetime { active: AtomicBool::new(true) }),
            configuration,
            ole32_policy,
            kernel32: None,
            ole32: None,
            winmm: None,
            advapi32: None,
            gdi32: None,
            private_raster: None,
            private_registry: None,
            winmm_clock: program_winmm::Clock::new(winmm_policy),
            modules_issued: 0,
            leases_issued: 0,
            observations_issued: 0,
            allocations: Vec::new(),
            live_global_bytes: 0,
            terminal: None,
        })
    }

    /// Opt in with an actual caller-owned private hive. Existing constructors
    /// leave ADVAPI32 disabled. Refusals return that same retained hive, including
    /// any uncertain write/poison state; no pathname is opened implicitly here.
    pub fn new_with_private_user_registry(
        memory: &Memory, configuration: Configuration,
        ole32: program_ole32::Policy, winmm: program_winmm::Policy,
        hive: program_registry_store::Hive, max_handles: usize,
    ) -> Result<Self, program_registry::BeginError> {
        let mut registry = match Self::new_with_profiles(memory, configuration, ole32, winmm) {
            Ok(registry) => registry,
            Err(error) => return Err(program_registry::BeginError { error, hive }),
        };
        registry.private_registry = Some(program_registry::Registry::new(
            memory, hive, program_registry::Policy::PrivateUserAsciiV1, max_handles)?);
        Ok(registry)
    }

    fn require_heap(&self, memory: &Memory) -> Result<(), Stop> {
        if !memory.has_identity(&self.heap) {
            return Err(unsupported("program registry belongs to another heap lifetime"));
        }
        Ok(())
    }
    fn require_live(&self, memory: &Memory) -> Result<(), Stop> {
        self.require_heap(memory)?;
        if !self.lifetime.active.load(Ordering::Acquire) {
            return Err(unsupported("program lifecycle is terminal"));
        }
        Ok(())
    }
    fn require_origin(&self, origin: &Weak<Lifetime>) -> Result<(), Stop> {
        if !Weak::ptr_eq(origin, &Arc::downgrade(&self.lifetime)) {
            return Err(unsupported("program receipt belongs to another registry lifetime"));
        }
        Ok(())
    }
    fn slot(&self, kind: ModuleKind) -> &Option<Arc<Module>> {
        match kind {
            ModuleKind::Kernel32 => &self.kernel32,
            ModuleKind::Ole32 => &self.ole32,
            ModuleKind::Winmm => &self.winmm,
            ModuleKind::Advapi32 => &self.advapi32,
            ModuleKind::Gdi32 => &self.gdi32,
        }
    }
    fn slot_mut(&mut self, kind: ModuleKind) -> &mut Option<Arc<Module>> {
        match kind {
            ModuleKind::Kernel32 => &mut self.kernel32,
            ModuleKind::Ole32 => &mut self.ole32,
            ModuleKind::Winmm => &mut self.winmm,
            ModuleKind::Advapi32 => &mut self.advapi32,
            ModuleKind::Gdi32 => &mut self.gdi32,
        }
    }
    fn loaded_module(&self, kind: ModuleKind) -> Result<&Arc<Module>, Stop> {
        self.slot(kind).as_ref().filter(|m| m.kind == kind && m.active.load(Ordering::Acquire))
            .ok_or_else(|| unsupported("the concrete Rust-port module is not loaded"))
    }
    fn require_policy(&self, kind: ModuleKind) -> Result<(), Stop> {
        match kind {
            ModuleKind::Gdi32 if self.private_raster.is_some() => Ok(()),
            ModuleKind::Advapi32 if self.private_registry.is_some() => Ok(()),
            ModuleKind::Kernel32 if self.configuration.kernel32 == Kernel32Policy::RustPort => Ok(()),
            ModuleKind::Ole32 if self.ole32_policy == program_ole32::Policy::OwnedRustSta => Ok(()),
            ModuleKind::Winmm if self.winmm_clock.policy() == program_winmm::Policy::DarwinKernelBootElapsed => Ok(()),
            _ => Err(unsupported("Rust-port module support was not explicitly enabled")),
        }
    }
    fn reserve_observation(&self) -> Result<(), Stop> {
        if self.observations_issued >= self.configuration.limits.weak_observation_issuances {
            return Err(exhausted("program weak-observation history is exhausted"));
        }
        Ok(())
    }
    #[cfg(test)]
    fn module_reference(&mut self) -> ModuleReference {
        self.module_reference_for(ModuleKind::Kernel32)
    }
    fn module_reference_for(&mut self, kind: ModuleKind) -> ModuleReference {
        let module = self.slot(kind).as_ref().expect("prechecked loaded module");
        let reference = ModuleReference {
            registry: Arc::downgrade(&self.lifetime),
            module: Arc::downgrade(module),
        };
        self.observations_issued += 1;
        reference
    }

    /// Explicitly load the concrete Rust implementation once; this does not
    /// load a host DLL. Repeated loading and unsupported policies refuse.
    pub fn load_kernel32(&mut self, memory: &Memory) -> Result<ModuleReference, Stop> {
        self.load_module(memory, ModuleKind::Kernel32)
    }
    pub fn load_ole32(&mut self, memory: &Memory) -> Result<ModuleReference, Stop> {
        self.load_module(memory, ModuleKind::Ole32)
    }
    pub fn load_winmm(&mut self, memory: &Memory) -> Result<ModuleReference, Stop> {
        self.load_module(memory, ModuleKind::Winmm)
    }
    pub fn load_advapi32(&mut self, memory: &Memory) -> Result<ModuleReference, Stop> {
        self.load_module(memory, ModuleKind::Advapi32)
    }
    pub fn load_module(&mut self, memory: &Memory, kind: ModuleKind) -> Result<ModuleReference, Stop> {
        self.require_live(memory)?;
        self.require_policy(kind)?;
        if self.slot(kind).is_some() {
            return Err(unsupported("concrete module is already loaded in this registry"));
        }
        if self.modules_issued >= self.configuration.limits.module_issuances {
            return Err(exhausted("program module issuance history is exhausted"));
        }
        self.reserve_observation()?;
        // All concrete table/identity allocation precedes publication. There
        // is no dynamically extensible callback or arbitrary export registry.
        let module = Arc::new(Module::new(kind));
        *self.slot_mut(kind) = Some(module);
        self.modules_issued += 1;
        Ok(self.module_reference_for(kind))
    }

    /// Weak observations never block retirement; any live import lease does.
    pub fn retire_kernel32(&mut self, memory: &Memory) -> Result<(), Stop> {
        self.retire_module(memory, ModuleKind::Kernel32)
    }
    pub fn retire_ole32(&mut self, memory: &Memory) -> Result<(), Stop> {
        self.retire_module(memory, ModuleKind::Ole32)
    }
    pub fn retire_winmm(&mut self, memory: &Memory) -> Result<(), Stop> {
        self.retire_module(memory, ModuleKind::Winmm)
    }
    pub fn retire_module(&mut self, memory: &Memory, kind: ModuleKind) -> Result<(), Stop> {
        self.require_live(memory)?;
        let module = self.loaded_module(kind)?;
        if kind == ModuleKind::Gdi32 && self.private_raster_observation(memory)?.live_contexts != 0 {
            return Err(unsupported("Gdi32 still has actual owning private raster contexts"));
        }
        if Arc::strong_count(module) != 1 {
            return Err(unsupported("module still has owning import procedure leases"));
        }
        module.active.store(false, Ordering::Release);
        *self.slot_mut(kind) = None;
        Ok(())
    }

    fn validate_name(&self, name: &[u8]) -> Result<(), Stop> {
        if name.is_empty() || name.len() > self.configuration.limits.max_name_bytes
            || !name.iter().all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(b))
            || name == b"." || name == b".."
        {
            return Err(unsupported("program name is outside the bounded ASCII basename profile"));
        }
        Ok(())
    }
    fn named_kind(&self, name: &[u8]) -> Result<Option<ModuleKind>, Stop> {
        self.validate_name(name)?;
        Ok(if name.eq_ignore_ascii_case(b"kernel32") || name.eq_ignore_ascii_case(b"kernel32.dll") {
            Some(ModuleKind::Kernel32)
        } else if name.eq_ignore_ascii_case(b"ole32") || name.eq_ignore_ascii_case(b"ole32.dll") {
            Some(ModuleKind::Ole32)
        } else if name.eq_ignore_ascii_case(b"advapi32") || name.eq_ignore_ascii_case(b"advapi32.dll") {
            Some(ModuleKind::Advapi32)
        } else if name.eq_ignore_ascii_case(b"gdi32") || name.eq_ignore_ascii_case(b"gdi32.dll") {
            Some(ModuleKind::Gdi32)
        } else if name.eq_ignore_ascii_case(b"winmm") || name.eq_ignore_ascii_case(b"winmm.dll") {
            Some(ModuleKind::Winmm)
        } else {
            None
        })
    }
    /// Resolve a bounded descriptor even when it has no symbols. This selects
    /// only a sealed enabled table; it neither loads nor grants live ownership.
    pub fn dependency_kind(&self, memory: &Memory, name: &[u8]) -> Result<ModuleKind, Stop> {
        self.require_live(memory)?;
        let kind = self.named_kind(name)?
            .ok_or_else(|| unsupported("only explicitly configured Rust-port dependencies are supported"))?;
        self.require_policy(kind)?;
        Ok(kind)
    }
    fn export_name<'a>(&self, query: ProcedureQuery<'a>) -> Result<&'a [u8], Stop> {
        match query {
            ProcedureQuery::Name(name) => {
                self.validate_name(name)?;
                Ok(name)
            }
            ProcedureQuery::Ordinal(_) => Err(unsupported("program export ordinal lookup is unsupported")),
        }
    }

    /// Explicit owning import issuance, independently authorized by parent.
    /// Lookup observations are not accepted as ownership or raw words.
    pub fn lease_import(
        &mut self,
        memory: &Memory,
        library: &[u8],
        query: ProcedureQuery<'_>,
    ) -> Result<ProcedureLease, Stop> {
        self.require_live(memory)?;
        let kind = self.dependency_kind(memory, library)?;
        let name = self.export_name(query)?;
        let module = self.loaded_module(kind)?;
        let procedure = module.find(name)
            .ok_or_else(|| unsupported("named import is absent from the actual module export table"))?;
        if procedure.export == Export::ExitProcess && self.configuration.exit == ExitPolicy::Disabled {
            return Err(unsupported("terminal Rust-port control policy was not explicitly enabled"));
        }
        if self.leases_issued >= self.configuration.limits.procedure_lease_issuances {
            return Err(exhausted("program import procedure history is exhausted"));
        }
        let lease = ProcedureLease {
            registry: Arc::downgrade(&self.lifetime),
            module: Arc::clone(module),
            procedure: Arc::clone(procedure),
        };
        self.leases_issued += 1;
        Ok(lease)
    }

    fn require_lease(&self, memory: &Memory, lease: &ProcedureLease) -> Result<Export, Stop> {
        self.require_live(memory)?;
        self.require_origin(&lease.registry)?;
        let module = self.loaded_module(lease.module.kind)?;
        if !Arc::ptr_eq(module, &lease.module) || !module.contains(&lease.procedure)
            || lease.procedure.export.module_kind() != module.kind
        {
            return Err(unsupported("import lost its actual module/procedure dependency"));
        }
        Ok(lease.procedure.export)
    }
    fn require_export(&self, memory: &Memory, access: ProcedureAccess<'_>, expected: Export) -> Result<(), Stop> {
        let actual = match access {
            ProcedureAccess::Import(lease) => self.require_lease(memory, lease)?,
            ProcedureAccess::Lookup(observation) => self.observed_export(memory, observation)?,
        };
        if actual != expected {
            return Err(unsupported("program capability names another concrete export"));
        }
        Ok(())
    }
    pub fn procedure_export(&self, memory: &Memory, lease: &ProcedureLease) -> Result<Export, Stop> {
        self.require_lease(memory, lease)
    }

    pub fn require_module(&self, memory: &Memory, reference: &ModuleReference) -> Result<(), Stop> {
        self.require_live(memory)?;
        self.require_origin(&reference.registry)?;
        self.referenced_module(&reference.module)?;
        Ok(())
    }
    fn referenced_module(&self, reference: &Weak<Module>) -> Result<&Arc<Module>, Stop> {
        // Compare weak identities without upgrading/pinning either table.
        [ModuleKind::Kernel32, ModuleKind::Ole32, ModuleKind::Winmm, ModuleKind::Advapi32, ModuleKind::Gdi32].into_iter()
            .filter_map(|kind| self.loaded_module(kind).ok())
            .find(|module| Weak::ptr_eq(reference, &Arc::downgrade(module)))
            .ok_or_else(|| unsupported("weak module reference is stale or replaced"))
    }
    pub fn module_identity(&self, memory: &Memory, reference: &ModuleReference) -> Result<ModuleIdentity, Stop> {
        self.require_module(memory, reference)?;
        let module = self.referenced_module(&reference.module)?;
        // A copyable observation of this already issued reference, not a new
        // live capability or ownership upgrade; no issuance history changes.
        Ok(ModuleIdentity { registry: reference.registry.clone(), module: reference.module.clone(), kind: module.kind })
    }
    pub(super) fn co_uninitialize_policy(&self, memory: &Memory, procedure: ProcedureAccess<'_>)
        -> Result<program_ole32::Policy, Stop>
    {
        self.require_export(memory, procedure, Export::CoUninitialize)?;
        self.require_policy(ModuleKind::Ole32)?;
        Ok(self.ole32_policy)
    }

    /// Authenticate the exact issuing heap, live registry, loaded WINMM table
    /// and actual owning/weak timeGetTime entry before the sole producer call.
    /// The producer retains its authentic Sample/reports before returning; no
    /// fallible publication, adoption, conversion of errors to zero or retry
    /// follows success. A genuine low DWORD zero is a successful return value.
    pub fn time_get_time(&mut self, memory: &Memory, procedure: ProcedureAccess<'_>) -> Result<u32, Stop> {
        self.require_export(memory, procedure, Export::TimeGetTime)?;
        self.require_policy(ModuleKind::Winmm)?;
        match self.winmm_clock.sample() {
            Ok(sample) => Ok(sample.value()),
            Err(stop) => Err(winmm_stop(stop)),
        }
    }

    /// Explicit additive enable on this actual begun registry. Authenticate
    /// before consuming the supplied owner; every refusal returns that Device.
    /// Old constructors and registry-only profiles retain Gdi32 disabled.
    pub fn enable_private_raster(
        &mut self, memory: &Memory, device: program_raster::Device,
        policy: program_raster_context::Policy, max_contexts: usize,
    ) -> Result<(), program_raster_context::BeginError> {
        if let Err(error) = self.require_live(memory) {
            return Err(program_raster_context::BeginError { error, device });
        }
        if self.private_raster.is_some() {
            return Err(program_raster_context::BeginError {
                error: unsupported("private raster was already enabled for this program"), device,
            });
        }
        let raster = program_raster_context::Registry::new(memory, device, policy, max_contexts)?;
        // No fallible publication or module issuance follows owner consumption.
        self.private_raster = Some(raster);
        Ok(())
    }

    fn raster(&self, memory: &Memory) -> Result<&program_raster_context::Registry, Stop> {
        self.require_live(memory)?;
        self.private_raster.as_ref()
            .ok_or_else(|| unsupported("private raster was not explicitly enabled"))
    }

    /// Private Rust helpers, not additional Win32 exports or raw HDC adoption.
    pub fn raster_attach_source(&mut self, memory: &Memory, bitmap: &program_raster::Bitmap)
        -> Result<program_raster_context::ContextLease, Stop>
    {
        self.require_live(memory)?;
        self.loaded_module(ModuleKind::Gdi32)?;
        self.private_raster.as_mut()
            .ok_or_else(|| unsupported("private raster was not explicitly enabled"))?
            .attach_source(memory, bitmap)
    }

    /// Logical dispatch through the actual table entry. None deliberately
    /// refuses: this program has no qualified current-screen source provider.
    pub fn create_compatible_dc(
        &mut self, memory: &Memory, procedure: ProcedureAccess<'_>,
        source: Option<&program_raster_context::ContextLease>,
    ) -> Result<program_raster_context::ContextLease, Stop> {
        self.require_export(memory, procedure, Export::CreateCompatibleDC)?;
        self.private_raster.as_mut()
            .ok_or_else(|| unsupported("private raster was not explicitly enabled"))?
            .create_compatible(memory, source)
    }

    pub fn raster_readback_rgba(&self, memory: &Memory, context: &program_raster_context::ContextLease)
        -> Result<program_raster::RgbaImage, Stop>
    {
        self.raster(memory)?.readback_rgba(memory, context)
    }

    pub fn raster_close(&mut self, memory: &Memory, context: &program_raster_context::ContextLease)
        -> Result<(), Stop>
    {
        self.require_live(memory)?;
        self.private_raster.as_mut()
            .ok_or_else(|| unsupported("private raster was not explicitly enabled"))?
            .close(memory, context)
    }

    /// A bounded state copy; it cannot issue a device, bitmap or context owner.
    pub fn private_raster_observation(&self, memory: &Memory)
        -> Result<program_raster_context::Observation, Stop>
    {
        self.raster(memory)?.snapshot(memory)
    }

    /// Typed registry operations, not guest ABI status or output-pointer stores.
    /// The exact live ADVAPI32 procedure and heap are checked before store I/O.
    pub fn reg_create_key_a(
        &mut self, memory: &Memory, procedure: ProcedureAccess<'_>,
        parent: program_registry::Parent<'_>, subkey: program_registry::SubKey<'_>,
    ) -> Result<program_registry::Created, Stop> {
        self.require_export(memory, procedure, Export::RegCreateKeyA)?;
        self.private_registry.as_mut()
            .ok_or_else(|| unsupported("the private user registry was not enabled"))?
            .create(memory, parent, subkey)
    }

    pub fn reg_close_key(
        &mut self, memory: &Memory, procedure: ProcedureAccess<'_>, key: &program_registry::KeyLease,
    ) -> Result<(), Stop> {
        self.require_export(memory, procedure, Export::RegCloseKey)?;
        self.private_registry.as_mut()
            .ok_or_else(|| unsupported("the private user registry was not enabled"))?
            .close(memory, key)
    }

    /// Memory-only last-confirmed file-store state, never key authority.
    pub fn private_registry_observation(&self, memory: &Memory)
        -> Result<program_registry_store::Observation, Stop>
    {
        self.require_heap(memory)?;
        self.private_registry.as_ref()
            .map(program_registry::Registry::store_snapshot)
            .ok_or_else(|| unsupported("the private user registry was not enabled"))
    }

    /// Heap-authenticated diagnostics, including after termination/retirement.
    /// Reports are evidence only; this grants no procedure or import authority.
    pub fn time_get_time_observation(&self, memory: &Memory) -> Result<TimeGetTimeObservation<'_>, Stop> {
        self.require_heap(memory)?;
        Ok(TimeGetTimeObservation { clock: &self.winmm_clock })
    }

    /// The supplied name excludes a terminating C NUL. Embedded NUL, paths,
    /// non-ASCII, oversize names and NULL executable lookup stop explicitly.
    pub fn get_module_handle_a(
        &mut self,
        memory: &Memory,
        procedure: ProcedureAccess<'_>,
        name: Option<&[u8]>,
    ) -> Result<Option<ModuleReference>, Stop> {
        self.require_export(memory, procedure, Export::GetModuleHandleA)?;
        let name = name.ok_or_else(|| unsupported("current-executable lookup has no owning executable registration"))?;
        let Some(kind) = self.named_kind(name)? else {
            // A valid basename is genuinely absent from this loaded table.
            return Ok(None);
        };
        if self.loaded_module(kind).is_err() { return Ok(None); }
        self.reserve_observation()?;
        Ok(Some(self.module_reference_for(kind)))
    }

    pub fn get_proc_address(
        &mut self,
        memory: &Memory,
        procedure: ProcedureAccess<'_>,
        module: &ModuleReference,
        query: ProcedureQuery<'_>,
    ) -> Result<Option<ProcedureObservation>, Stop> {
        self.require_export(memory, procedure, Export::GetProcAddress)?;
        self.require_module(memory, module)?;
        let name = self.export_name(query)?;
        let loaded = self.referenced_module(&module.module)?;
        let Some(procedure) = loaded.find(name) else {
            return Ok(None);
        };
        self.reserve_observation()?;
        let observation = ProcedureObservation {
            registry: Arc::downgrade(&self.lifetime),
            module: Arc::downgrade(loaded),
            procedure: Arc::downgrade(procedure),
        };
        self.observations_issued += 1;
        Ok(Some(observation))
    }

    pub fn observed_export(&self, memory: &Memory, observation: &ProcedureObservation) -> Result<Export, Stop> {
        self.require_live(memory)?;
        self.require_origin(&observation.registry)?;
        let module = self.referenced_module(&observation.module)?;
        module.procedures.iter()
            .find(|p| p.export.module_kind() == module.kind
                && Weak::ptr_eq(&observation.procedure, &Arc::downgrade(p)))
            .map(|p| p.export)
            .ok_or_else(|| unsupported("weak procedure identity is stale or replaced"))
    }

    pub fn global_alloc(
        &mut self,
        memory: &mut Memory,
        procedure: ProcedureAccess<'_>,
        flags: u32,
        bytes: u32,
    ) -> Result<GlobalAllocReturn, Stop> {
        self.require_export(memory, procedure, Export::GlobalAlloc)?;
        if !matches!(flags, 0 | 0x40) || bytes == 0 {
            return Err(unsupported("GlobalAlloc supports only positive fixed flags 0 or 0x40"));
        }
        let limits = self.configuration.limits;
        if bytes > limits.max_global_bytes {
            return Err(unsupported("GlobalAlloc extent exceeds the configured host profile"));
        }
        let next_live = self.live_global_bytes.checked_add(bytes as usize)
            .ok_or_else(|| exhausted("program global byte accounting overflows"))?;
        if next_live > limits.max_live_global_bytes || self.allocations.len() >= limits.global_allocation_issuances {
            return Err(exhausted("program global allocation history or live-byte profile is exhausted"));
        }
        self.allocations.try_reserve_exact(1)
            .map_err(|_| exhausted("program global issuance ledger could not reserve memory"))?;
        // The opaque owner and all fallible ledger capacity exist before the
        // real allocation. Nothing fallible follows a valid hook success.
        let issuance = Arc::new(AllocationIdentity);
        let owner = GlobalAllocation {
            registry: Arc::downgrade(&self.lifetime),
            issuance: Arc::clone(&issuance),
        };
        let allocated = match memory.allocate_program_global(bytes) {
            Ok(allocated) => allocated,
            Err(error) => return allocation_error(error),
        };
        let Allocated::Bytes { token, data } = allocated else {
            panic!("typed ProgramGlobal allocator must classify OOM, never return Ok(Null)");
        };
        // These are the parent hook's success invariants, not error-string
        // heuristics or an alternative arbitrary-address registration path.
        let pointer = u32::try_from(token).expect("ProgramGlobal hook returned a PE32 pointer");
        assert!(pointer != 0 && pointer % 8 == 0, "ProgramGlobal hook alignment/NULL contract");
        assert_eq!(memory.allocation_kind(token).expect("hook issued live storage"), Kind::ProgramGlobal);
        let actual = memory.allocation_bytes(token).expect("hook issued a complete allocation");
        assert_eq!(actual.len(), bytes as usize, "ProgramGlobal hook extent contract");
        assert_eq!(data.as_slice(), actual, "ProgramGlobal hook initialized image contract");
        assert!(actual.iter().all(|byte| *byte == 0), "explicit Rust-port zero initialization policy");
        self.allocations.push(AllocationRecord { issuance, pointer, bytes, live: true });
        self.live_global_bytes = next_live;
        Ok(GlobalAllocReturn::Allocated(owner))
    }

    fn allocation_index(&self, owner: &GlobalAllocation) -> Result<usize, Stop> {
        self.require_origin(&owner.registry)?;
        self.allocations.iter().position(|r| Arc::ptr_eq(&r.issuance, &owner.issuance))
            .ok_or_else(|| unsupported("global owner was not issued by this registry"))
    }
    fn require_storage(&self, memory: &Memory, record: &AllocationRecord) -> Result<(), Stop> {
        if memory.allocation_kind(u64::from(record.pointer))? != Kind::ProgramGlobal
            || memory.allocation_bytes(u64::from(record.pointer))?.len() != record.bytes as usize
        {
            return Err(unsupported("issued global storage changed its kind or complete extent"));
        }
        Ok(())
    }
    pub fn allocation_word(&self, memory: &Memory, owner: &GlobalAllocation) -> Result<u32, Stop> {
        self.require_live(memory)?;
        let record = &self.allocations[self.allocation_index(owner)?];
        if !record.live {
            return Err(unsupported("global allocation owner has already been released"));
        }
        self.require_storage(memory, record)?;
        Ok(record.pointer)
    }

    /// Arbitrary handles cannot be adopted. A mismatching/interior word,
    /// repeated release or wrong kind is a failure returning the original word;
    /// foreign heap/registry and externally missing/changed extents are host
    /// refusals. No refusal is silently converted to native NULL success.
    pub fn global_free(
        &mut self,
        memory: &mut Memory,
        procedure: ProcedureAccess<'_>,
        owner: &GlobalAllocation,
        handle: u32,
    ) -> Result<GlobalFreeReturn, Stop> {
        self.require_export(memory, procedure, Export::GlobalFree)?;
        let index = self.allocation_index(owner)?;
        let record = &self.allocations[index];
        let failure = if handle != record.pointer {
            Some(FreeFailure::HandleMismatch)
        } else if !record.live {
            Some(FreeFailure::AlreadyReleased)
        } else if memory.allocation_kind(u64::from(record.pointer))? != Kind::ProgramGlobal {
            Some(FreeFailure::WrongAllocationKind)
        } else {
            None
        };
        if let Some(reason) = failure {
            return Ok(GlobalFreeReturn::Failed { original_handle: handle, reason });
        }
        self.require_storage(memory, record)?;
        memory.release(u64::from(record.pointer), Kind::ProgramGlobal)
            .expect("exclusively borrowed complete issued global was prevalidated");
        self.live_global_bytes -= record.bytes as usize;
        self.allocations[index].live = false;
        Ok(GlobalFreeReturn::Released)
    }

    /// Seal first-class program control and actually release this registry's
    /// remaining globals. All storage is prevalidated before the first free.
    /// Surviving import leases become unusable even though they retain tables.
    pub fn exit_process(
        &mut self,
        memory: &mut Memory,
        procedure: ProcedureAccess<'_>,
        exit_code: u32,
    ) -> Result<TerminalTransfer, Stop> {
        self.require_export(memory, procedure, Export::ExitProcess)?;
        if self.configuration.exit != ExitPolicy::SingleThreadNoGuestDllDetachCallbacks {
            return Err(unsupported("ExitProcess requires the restricted terminal Rust-port policy"));
        }
        let mut released_globals = 0;
        for record in self.allocations.iter().filter(|r| r.live) {
            self.require_storage(memory, record)?;
            released_globals += 1;
        }
        let terminal = Arc::new(TerminalIdentity { exit_code, released_globals });
        let transfer = TerminalTransfer {
            registry: Arc::downgrade(&self.lifetime),
            terminal: Arc::clone(&terminal),
        };
        // Exclusive Memory and Registry borrows keep preflight valid throughout
        // this non-callback commit. There is no ordinary guest return afterward.
        self.lifetime.active.store(false, Ordering::Release);
        if let Some(module) = self.kernel32.take() {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = self.ole32.take() {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = self.winmm.take() {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = self.advapi32.take() {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = self.gdi32.take() {
            module.active.store(false, Ordering::Release);
        }
        // Drop the actual registry now, even if context receipts remain alive.
        self.private_raster = None;
        // Revokes every key epoch; persistent key records survive normal close.
        self.private_registry = None;
        for record in self.allocations.iter_mut().filter(|r| r.live) {
            memory.release(u64::from(record.pointer), Kind::ProgramGlobal)
                .expect("terminal preflight validated every exact owned global");
            record.live = false;
        }
        self.live_global_bytes = 0;
        self.terminal = Some(terminal);
        Ok(transfer)
    }

    /// Consume the exact terminal transfer once. A rejected transfer never
    /// drops/seals an otherwise live registry. On success the registry stays
    /// sealed; parent must propagate `Termination` through its outer guest
    /// control path and may then drop this registry and its capabilities.
    pub fn finish(&mut self, memory: &Memory, transfer: TerminalTransfer) -> Result<Termination, Stop> {
        self.require_heap(memory)?;
        self.require_origin(&transfer.registry)?;
        if self.lifetime.active.load(Ordering::Acquire) || self.terminal.as_ref()
            .is_none_or(|terminal| !Arc::ptr_eq(terminal, &transfer.terminal))
        {
            return Err(unsupported("terminal transfer is stale or belongs to another program seal"));
        }
        let termination = Termination {
            exit_code: transfer.terminal.exit_code,
            released_globals: transfer.terminal.released_globals,
        };
        self.terminal = None;
        Ok(termination)
    }

    pub fn history(&self, memory: &Memory) -> Result<History, Stop> {
        self.require_live(memory)?;
        Ok(History {
            modules_issued: self.modules_issued,
            procedure_leases_issued: self.leases_issued,
            weak_observations_issued: self.observations_issued,
            global_allocations_issued: self.allocations.len(),
            live_global_bytes: self.live_global_bytes,
        })
    }
}

fn allocation_error(error: ProgramAllocationError) -> Result<GlobalAllocReturn, Stop> {
    match error {
        ProgramAllocationError::Refused(stop) => Err(stop),
        ProgramAllocationError::OutOfMemory => Ok(GlobalAllocReturn::OutOfMemory),
    }
}

fn winmm_stop(stop: program_winmm::Stop) -> Stop {
    use program_winmm::Stop as ClockStop;
    // The exact typed reason remains in Clock's immutable reports. Service
    // Stop has no clock variant; classify every producer refusal explicitly.
    match stop {
        ClockStop::Disabled | ClockStop::UnsupportedHost | ClockStop::ForeignSample =>
            unsupported(&format!("WINMM timeGetTime producer refused: {stop:?}")),
        ClockStop::SequenceExhausted | ClockStop::NativeCallFailed { .. }
            | ClockStop::UnexpectedLength { .. } | ClockStop::InvalidScalar { .. }
            | ClockStop::BootChanged | ClockStop::CalendarBeforeBoot
            | ClockStop::ElapsedRegression { .. } =>
            exhausted(&format!("WINMM timeGetTime producer refused: {stop:?}")),
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.lifetime.active.store(false, Ordering::Release);
        if let Some(module) = &self.kernel32 {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = &self.ole32 {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = &self.winmm {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = &self.advapi32 {
            module.active.store(false, Ordering::Release);
        }
        if let Some(module) = &self.gdi32 {
            module.active.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests;
