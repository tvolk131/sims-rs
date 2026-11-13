//! Concrete, owner-bound program services. This layer neither patches imports
//! nor admits an ABI frame, CPU/stack, driver, constructor or Application.
//! The caller retains its executable Module (or Entry) and presents it on every
//! operation. An origin observation or repeating native word cannot replace it.

use super::{
    BootstrapServices, Capability, Identity, Memory, ProgramTermination, Stop,
    program_modules::{self, Export, ModuleIdentity, ModuleKind, ProcedureAccess, ProcedureQuery},
    program_ole32, program_raster, program_raster_context, program_registry, program_registry_store, program_winmm,
    unsupported,
};
use crate::native_bootstrap::executable::{Module, ModuleOrigin};
use std::{collections::BTreeMap, sync::Arc};

struct Issuance;

/// Privately issued authority for exactly one service/program lifetime and its
/// actual executable input. No Clone, serialization, integer adoption or public
/// constructor. Retaining this value does not replace the borrowed Module.
///
/// ```compile_fail
/// use sims_content::native_bootstrap::services::ProgramAccess;
/// fn copy(access: &ProgramAccess) -> ProgramAccess { access.clone() }
/// ```
/// ```compile_fail
/// use sims_content::native_bootstrap::services::ProgramAccess;
/// let _ = ProgramAccess { issuance: std::sync::Arc::new(()), origin: 0 };
/// ```
/// An origin observation cannot substitute for the caller's actual owner:
/// ```compile_fail
/// use sims_content::native_bootstrap::{executable::ModuleOrigin,
///     services::{BootstrapServices, ProgramAccess}};
/// fn call(host: &mut BootstrapServices<'_>, access: &ProgramAccess, origin: &ModuleOrigin) {
///     let _ = host.program_global_alloc(access, origin, 1, 0, 8);
/// }
/// ```
pub struct ProgramAccess {
    issuance: Arc<Issuance>,
    origin: ModuleOrigin,
}

struct GlobalReceipt {
    pointer: u32,
    owner: program_modules::GlobalAllocation,
}

pub(super) struct Program {
    issuance: Arc<Issuance>,
    origin: ModuleOrigin,
    heap: Identity,
    registry: program_modules::Registry,
    globals: Vec<GlobalReceipt>,
    receipt_limit: usize,
    sealed: bool,
}

fn procedure<'a>(
    capabilities: &'a BTreeMap<u32, Capability>,
    registry: &program_modules::Registry,
    memory: &Memory,
    word: u32,
    expected: Export,
) -> Result<ProcedureAccess<'a>, Stop> {
    let access = match capabilities.get(&word) {
        Some(Capability::ProgramImport(lease)) => Ok(ProcedureAccess::Import(lease)),
        Some(Capability::ProgramProcedure(observation)) => Ok(ProcedureAccess::Lookup(observation)),
        Some(_) => Err(unsupported("program dispatch requires an import or weak procedure capability")),
        None => Err(unsupported("program procedure capability is not live")),
    }?;
    let actual = match access {
        ProcedureAccess::Import(lease) => registry.procedure_export(memory, lease)?,
        ProcedureAccess::Lookup(observation) => registry.observed_export(memory, observation)?,
    };
    if actual != expected {
        return Err(unsupported("program dispatch names another concrete export"));
    }
    Ok(access)
}

fn module(
    capabilities: &BTreeMap<u32, Capability>,
    word: u32,
) -> Result<&program_modules::ModuleReference, Stop> {
    match capabilities.get(&word) {
        Some(Capability::ProgramModule(reference)) => Ok(reference),
        Some(_) => Err(unsupported("program lookup requires a weak module capability")),
        None => Err(unsupported("program module capability is not live")),
    }
}

fn registry_root(word: u32) -> Option<program_registry::PredefinedRoot> {
    use program_registry::PredefinedRoot;
    if word == PredefinedRoot::ClassesRoot.word() { Some(PredefinedRoot::ClassesRoot) }
    else if word == PredefinedRoot::CurrentUser.word() { Some(PredefinedRoot::CurrentUser) }
    else if word == PredefinedRoot::LocalMachine.word() { Some(PredefinedRoot::LocalMachine) }
    else if word == PredefinedRoot::Users.word() { Some(PredefinedRoot::Users) }
    else if word == PredefinedRoot::CurrentConfig.word() { Some(PredefinedRoot::CurrentConfig) }
    else { None }
}

fn raster_context(capabilities: &BTreeMap<u32, Capability>, word: u32)
    -> Result<&program_raster_context::ContextLease, Stop>
{
    match capabilities.get(&word) {
        Some(Capability::ProgramRasterContext(context)) => Ok(context),
        _ => Err(unsupported("private raster requires an actual issued context capability")),
    }
}

impl BootstrapServices<'_> {
    /// Register one actual input with explicit policies. No module is installed
    /// until program_load_kernel32 is called. A refused configuration leaves
    /// this service unregistered; an existing lifetime can never be replaced,
    /// including after termination or a heap swap.
    pub fn begin_program(
        &mut self,
        input: &Module,
        configuration: program_modules::Configuration,
    ) -> Result<ProgramAccess, Stop> {
        self.begin_program_with_ole32(input, configuration, program_ole32::Policy::Disabled)
    }

    /// Explicit OLE32 policy for a fresh program only. Old callers retain COM
    /// disabled, and no path can replace/reconfigure a registered program.
    pub fn begin_program_with_ole32(
        &mut self,
        input: &Module,
        configuration: program_modules::Configuration,
        ole32: program_ole32::Policy,
    ) -> Result<ProgramAccess, Stop> {
        self.begin_program_with_profiles(input, configuration, ole32, program_winmm::Policy::Disabled)
    }

    /// Explicit profiles for a fresh program only. One registry-owned Clock
    /// survives every transient module load; registration itself does no I/O.
    pub fn begin_program_with_profiles(
        &mut self,
        input: &Module,
        configuration: program_modules::Configuration,
        ole32: program_ole32::Policy,
        winmm: program_winmm::Policy,
    ) -> Result<ProgramAccess, Stop> {
        if self.program.is_some() {
            return Err(unsupported("bootstrap services already issued their program lifetime"));
        }
        let registry = program_modules::Registry::new_with_profiles(&self.memory, configuration, ole32, winmm)?;
        Ok(self.register_program(input, configuration, registry))
    }

    /// Explicit private-user ASCII file-backed profile. Every refusal retains
    /// the actual hive in BeginError; no live program can be reconfigured.
    pub fn begin_program_with_private_user_registry(
        &mut self, input: &Module, configuration: program_modules::Configuration,
        ole32: program_ole32::Policy, winmm: program_winmm::Policy,
        hive: program_registry_store::Hive, max_handles: usize,
    ) -> Result<ProgramAccess, program_registry::BeginError> {
        if self.program.is_some() {
            return Err(program_registry::BeginError {
                error: unsupported("bootstrap services already issued their program lifetime"), hive,
            });
        }
        let registry = program_modules::Registry::new_with_private_user_registry(
            &self.memory, configuration, ole32, winmm, hive, max_handles)?;
        Ok(self.register_program(input, configuration, registry))
    }

    fn register_program(&mut self, input: &Module, configuration: program_modules::Configuration,
                        registry: program_modules::Registry) -> ProgramAccess {
        let issuance = Arc::new(Issuance);
        let origin = input.origin();
        let access = ProgramAccess {
            issuance: Arc::clone(&issuance),
            origin: origin.clone(),
        };
        self.program = Some(Program {
            issuance,
            origin,
            heap: self.memory.identity(),
            registry,
            globals: Vec::new(),
            receipt_limit: configuration.limits.global_allocation_issuances,
            sealed: false,
        });
        access
    }

    fn require_program(&self, access: &ProgramAccess, input: &Module) -> Result<(), Stop> {
        self.require_program_owner(access, input)?;
        if self.program.as_ref().expect("authenticated program").sealed {
            return Err(unsupported("program lifecycle is terminal"));
        }
        Ok(())
    }

    // Diagnostics retain exact issuer/input/heap authentication after sealing;
    // every effectful API additionally requires the live program above.
    fn require_program_owner(&self, access: &ProgramAccess, input: &Module) -> Result<(), Stop> {
        let program = self.program.as_ref()
            .ok_or_else(|| unsupported("no program lifetime was explicitly registered"))?;
        if !Arc::ptr_eq(&program.issuance, &access.issuance)
            || program.origin != access.origin
            || program.origin != input.origin()
            || !self.memory.has_identity(&program.heap)
        {
            return Err(unsupported("program access, input owner, heap or lifetime differs from its issuer"));
        }
        Ok(())
    }

    /// Issue a logical weak-module capability for the concrete configured Rust
    /// port. The shared monotonic allocator is checked before backend issuance.
    pub fn program_load_kernel32(
        &mut self,
        access: &ProgramAccess,
        input: &Module,
    ) -> Result<u32, Stop> {
        self.program_load_module(access, input, ModuleKind::Kernel32)
    }

    pub fn program_load_ole32(&mut self, access: &ProgramAccess, input: &Module) -> Result<u32, Stop> {
        self.program_load_module(access, input, ModuleKind::Ole32)
    }

    pub fn program_load_winmm(&mut self, access: &ProgramAccess, input: &Module) -> Result<u32, Stop> {
        self.program_load_module(access, input, ModuleKind::Winmm)
    }

    /// Additive opt-in on the actual live program/input/heap. All authentication
    /// precedes Device consumption; every refusal returns that same owner.
    pub fn program_enable_private_raster(
        &mut self, access: &ProgramAccess, input: &Module, device: program_raster::Device,
        policy: program_raster_context::Policy, max_contexts: usize,
    ) -> Result<(), program_raster_context::BeginError> {
        if let Err(error) = self.require_program(access, input) {
            return Err(program_raster_context::BeginError { error, device });
        }
        self.program.as_mut().expect("authenticated program").registry
            .enable_private_raster(&self.memory, device, policy, max_contexts)
    }

    /// Private source attachment uses a genuine caller Bitmap. The shared
    /// monotonic capability budget is reserved before context/device effects.
    pub fn program_raster_attach_source(
        &mut self, access: &ProgramAccess, input: &Module, bitmap: &program_raster::Bitmap,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let word = self.next_capability()?;
        let context = self.program.as_mut().expect("authenticated program").registry
            .raster_attach_source(&self.memory, bitmap)?;
        self.commit(word, Capability::ProgramRasterContext(context));
        Ok(word)
    }

    /// Logical CreateCompatibleDC dispatch, not a Win32 HDC/ABI return. Zero
    /// forwards None to the real producer and refuses without a screen claim.
    /// Raw words resolve only within this authenticated program's namespace.
    pub fn program_create_compatible_dc(
        &mut self, access: &ProgramAccess, input: &Module,
        procedure_word: u32, source_context_word: u32,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let word = self.next_capability()?;
        let program = self.program.as_mut().expect("authenticated program");
        let procedure = procedure(&self.capabilities, &program.registry, &self.memory,
                                  procedure_word, Export::CreateCompatibleDC)?;
        let source = if source_context_word == 0 { None }
            else { Some(raster_context(&self.capabilities, source_context_word)?) };
        let context = program.registry.create_compatible_dc(&self.memory, procedure, source)?;
        self.commit(word, Capability::ProgramRasterContext(context));
        Ok(word)
    }

    pub fn program_raster_readback_rgba(
        &self, access: &ProgramAccess, input: &Module, context_word: u32,
    ) -> Result<program_raster::RgbaImage, Stop> {
        self.require_program(access, input)?;
        let context = raster_context(&self.capabilities, context_word)?;
        self.program.as_ref().expect("authenticated program").registry
            .raster_readback_rgba(&self.memory, context)
    }

    /// Revoke this exact issued use once. Caller Bitmap ownership survives;
    /// the context's selection/default are actually released by the producer.
    pub fn program_raster_close(
        &mut self, access: &ProgramAccess, input: &Module, context_word: u32,
    ) -> Result<(), Stop> {
        self.require_program(access, input)?;
        let context = raster_context(&self.capabilities, context_word)?;
        self.program.as_mut().expect("authenticated program").registry
            .raster_close(&self.memory, context)?;
        self.capabilities.remove(&context_word);
        Ok(())
    }

    pub fn program_private_raster_observation(
        &self, access: &ProgramAccess, input: &Module,
    ) -> Result<program_raster_context::Observation, Stop> {
        self.require_program(access, input)?;
        self.program.as_ref().expect("authenticated program").registry
            .private_raster_observation(&self.memory)
    }

    pub fn program_load_module(
        &mut self, access: &ProgramAccess, input: &Module, kind: ModuleKind,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let word = self.next_capability()?;
        let program = self.program.as_mut().expect("authenticated program");
        let reference = program.registry.load_module(&self.memory, kind)?;
        self.commit(word, Capability::ProgramModule(reference));
        Ok(word)
    }

    /// Return a borrowed predefined root or an actually issued key capability.
    /// This is a logical HKEY result, not LSTATUS or a guest output-memory store.
    /// Program, executable, heap, procedure and parent checks precede file effects.
    pub fn program_reg_create_key_a(
        &mut self, access: &ProgramAccess, input: &Module, procedure_word: u32,
        parent_word: u32, subkey: program_registry::SubKey<'_>,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let root = registry_root(parent_word);
        // Borrowed root/null results allocate nothing and need no capability
        // budget. All owned results reserve the shared word before file writes.
        let reserved = if root.is_some() && matches!(subkey, program_registry::SubKey::Null) {
            None
        } else {
            Some(self.next_capability()?)
        };
        let program = self.program.as_mut().expect("authenticated program");
        let procedure = procedure(&self.capabilities, &program.registry, &self.memory,
                                  procedure_word, Export::RegCreateKeyA)?;
        let parent = match root {
            Some(root) => program_registry::Parent::Predefined(root),
            None => match self.capabilities.get(&parent_word) {
                Some(Capability::ProgramRegistryKey(key)) => program_registry::Parent::Key(key),
                _ => return Err(unsupported("registry parent has no actual owned key capability")),
            },
        };
        match program.registry.reg_create_key_a(&self.memory, procedure, parent, subkey)? {
            program_registry::Created::BorrowedRoot(root) => Ok(root.word()),
            program_registry::Created::Owned(key) => {
                let word = reserved.expect("an owned key requires the prechecked capability word");
                self.commit(word, Capability::ProgramRegistryKey(key));
                Ok(word)
            }
        }
    }

    /// Write the actual logical HKEY into four little-endian bytes of one live
    /// current-program GlobalAlloc buffer, preserving its surrounding bytes.
    /// The retained GlobalAllocation receipt, not an address/Kind/Owner image,
    /// authenticates the complete storage before any registry effects.
    ///
    /// This host-service consumer does not qualify guest CStr marshalling,
    /// stack/ABI admission, LSTATUS, output faults, original callers or native
    /// success. A real file failure retains the existing poisoned hive; this
    /// method promises unchanged output on refusal, not file rollback.
    pub fn program_reg_create_key_a_into(
        &mut self, access: &ProgramAccess, input: &Module, procedure_word: u32,
        parent_word: u32, subkey: program_registry::SubKey<'_>, result_address: u32,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let result_end = result_address.checked_add(4)
            .filter(|_| result_address != 0)
            .ok_or_else(|| unsupported("registry output span is null or overflows PE32"))?;
        let program = self.program.as_ref().expect("authenticated program");
        // Selection alone conveys no authority. The genuine retained receipt
        // below must still authenticate its registry, active issuance and exact
        // live ProgramGlobal storage in this program's already-checked heap.
        let receipt = program.globals.iter()
            .filter(|receipt| receipt.pointer <= result_address)
            .max_by_key(|receipt| receipt.pointer)
            .ok_or_else(|| unsupported("registry output has no retained program-global receipt"))?;
        let pointer = program.registry.allocation_word(&self.memory, &receipt.owner)?;
        if pointer != receipt.pointer {
            return Err(unsupported("registry output receipt differs from its issued pointer"));
        }
        let actual = self.memory.allocation_bytes(u64::from(pointer))?;
        let bytes = u32::try_from(actual.len())
            .map_err(|_| unsupported("registry output allocation extent exceeds PE32"))?;
        let allocation_end = pointer.checked_add(bytes)
            .ok_or_else(|| unsupported("registry output allocation end overflows PE32"))?;
        if result_end > allocation_end {
            return Err(unsupported("registry output crosses its complete program-global allocation"));
        }
        let offset = result_address.checked_sub(pointer)
            .ok_or_else(|| unsupported("registry output precedes its program-global allocation"))? as usize;
        let mut data = Vec::new();
        data.try_reserve_exact(actual.len())
            .map_err(|_| Stop::Unavailable("registry output image could not reserve memory".into()))?;
        data.extend_from_slice(actual);
        // This private full image is synchronization data, never owner proof.
        // All fallible output preparation is complete before the real key call.
        let mut image = super::Owner { token: u64::from(pointer), data };
        let word = self.program_reg_create_key_a(access, input, procedure_word, parent_word, subkey)?;
        image.data[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        // Under this exclusive service borrow the key operation neither mutates
        // global Memory nor invokes Rust callbacks. The authenticated allocation,
        // kind and full extent therefore remain exact; publication only copies
        // into existing storage and has no allocation or recoverable failure.
        self.memory.synchronize_owner(&image)
            .expect("prevalidated program-global output stays exact across the exclusive key operation");
        Ok(word)
    }

    /// Revoke the real owned handle without deleting the persistent key.
    /// Predefined roots are borrowed, and are never routed to an owned close.
    pub fn program_reg_close_key(
        &mut self, access: &ProgramAccess, input: &Module, procedure_word: u32, key_word: u32,
    ) -> Result<(), Stop> {
        self.require_program(access, input)?;
        let program = self.program.as_mut().expect("authenticated program");
        let procedure = procedure(&self.capabilities, &program.registry, &self.memory,
                                  procedure_word, Export::RegCloseKey)?;
        let key = match self.capabilities.get(&key_word) {
            Some(Capability::ProgramRegistryKey(key)) => key,
            _ => return Err(unsupported("registry close requires an actual owned key capability")),
        };
        program.registry.reg_close_key(&self.memory, procedure, key)?;
        self.capabilities.remove(&key_word);
        Ok(())
    }

    pub fn program_private_registry_observation(&self, access: &ProgramAccess, input: &Module)
        -> Result<program_registry_store::Observation, Stop>
    {
        self.require_program_owner(access, input)?;
        self.program.as_ref().expect("authenticated program").registry.private_registry_observation(&self.memory)
    }

    /// Typed, bounded descriptor resolution, including zero-symbol libraries.
    pub fn program_dependency_kind(&self, access: &ProgramAccess, input: &Module, library: &[u8])
        -> Result<ModuleKind, Stop>
    {
        self.require_program(access, input)?;
        self.program.as_ref().expect("authenticated program").registry.dependency_kind(&self.memory, library)
    }

    pub fn program_module_identity(&self, access: &ProgramAccess, input: &Module, module_word: u32)
        -> Result<ModuleIdentity, Stop>
    {
        self.require_program(access, input)?;
        let reference = module(&self.capabilities, module_word)?;
        self.program.as_ref().expect("authenticated program").registry.module_identity(&self.memory, reference)
    }

    /// Void balance of the exact issuing-service Rust STA Return. Every
    /// program/input/Memory/procedure/policy/application/Return check precedes
    /// the producer; no publication, capability removal or fallible work follows
    /// its effect. Immutable apartment history stays retained after release.
    pub fn program_co_uninitialize(
        &self, access: &ProgramAccess, input: &Module, procedure_word: u32,
        returned: &sims_sim::native_application::com::Return,
    ) -> Result<(), Stop> {
        self.require_program(access, input)?;
        let registry = &self.program.as_ref().expect("authenticated program").registry;
        let procedure = procedure(&self.capabilities, registry, &self.memory, procedure_word, Export::CoUninitialize)?;
        let policy = registry.co_uninitialize_policy(&self.memory, procedure)?;
        let held = self.apartment_return(returned)?;
        program_ole32::uninitialize(policy, held)
    }

    /// Return the authentic retained Clock Sample's low DWORD, including zero.
    /// All owner/input/heap/capability checks precede the one backend attempt;
    /// producer failures remain service errors with typed read-only diagnostics.
    pub fn program_time_get_time(
        &mut self, access: &ProgramAccess, input: &Module, procedure_word: u32,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let program = self.program.as_mut().expect("authenticated program");
        let procedure = procedure(&self.capabilities, &program.registry, &self.memory,
            procedure_word, Export::TimeGetTime)?;
        program.registry.time_get_time(&self.memory, procedure)
    }

    /// Bounded immutable evidence from the same registry-owned Clock. Exact
    /// access, input and Memory remain mandatory, including after termination.
    /// No observation can mutate the Clock or be adopted as a new sample.
    pub fn program_time_get_time_observation(
        &self, access: &ProgramAccess, input: &Module,
    ) -> Result<program_modules::TimeGetTimeObservation<'_>, Stop> {
        self.require_program_owner(access, input)?;
        self.program.as_ref().expect("authenticated program").registry.time_get_time_observation(&self.memory)
    }

    /// Explicit owning import issuance. Lookup words are never upgraded into
    /// imports. Names, ordinals and policies retain the concrete registry rules.
    pub fn program_lease_import(
        &mut self,
        access: &ProgramAccess,
        input: &Module,
        library: &[u8],
        query: ProcedureQuery<'_>,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let word = self.next_capability()?;
        let program = self.program.as_mut().expect("authenticated program");
        let lease = program.registry.lease_import(&self.memory, library, query)?;
        self.commit(word, Capability::ProgramImport(lease));
        Ok(word)
    }

    /// Names omit the terminating C NUL. Zero is returned only for a genuine
    /// backend absence; unsupported input and profile stops remain Err(Stop).
    pub fn program_get_module_handle_a(
        &mut self,
        access: &ProgramAccess,
        input: &Module,
        procedure_word: u32,
        name: Option<&[u8]>,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let procedure = procedure(
            &self.capabilities, &self.program.as_ref().expect("authenticated program").registry,
            &self.memory, procedure_word, Export::GetModuleHandleA,
        )?;
        let word = self.next_capability()?;
        let program = self.program.as_mut().expect("authenticated program");
        let Some(reference) = program.registry.get_module_handle_a(&self.memory, procedure, name)? else {
            return Ok(0);
        };
        self.commit(word, Capability::ProgramModule(reference));
        Ok(word)
    }

    /// A successful lookup issues a weak procedure observation, never an
    /// owning import lease or a native/Rust function address.
    pub fn program_get_proc_address(
        &mut self,
        access: &ProgramAccess,
        input: &Module,
        procedure_word: u32,
        module_word: u32,
        query: ProcedureQuery<'_>,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let procedure = procedure(
            &self.capabilities, &self.program.as_ref().expect("authenticated program").registry,
            &self.memory, procedure_word, Export::GetProcAddress,
        )?;
        let word = self.next_capability()?;
        let reference = module(&self.capabilities, module_word)?;
        let program = self.program.as_mut().expect("authenticated program");
        let Some(observation) = program.registry.get_proc_address(
            &self.memory, procedure, reference, query,
        )? else {
            return Ok(0);
        };
        self.commit(word, Capability::ProgramProcedure(observation));
        Ok(word)
    }

    /// Return the actual issued Memory pointer, retaining its genuine receipt.
    /// Only the registry's typed OutOfMemory becomes zero. Receipt reservations
    /// and all host/profile refusals remain errors. Recoverable Vec reservation
    /// failures are covered; Arc/BTreeMap metadata may still abort on host OOM.
    pub fn program_global_alloc(
        &mut self,
        access: &ProgramAccess,
        input: &Module,
        procedure_word: u32,
        flags: u32,
        bytes: u32,
    ) -> Result<u32, Stop> {
        self.require_program(access, input)?;
        let procedure = procedure(
            &self.capabilities, &self.program.as_ref().expect("authenticated program").registry,
            &self.memory, procedure_word, Export::GlobalAlloc,
        )?;
        let program = self.program.as_mut().expect("authenticated program");
        if program.globals.len() >= program.receipt_limit {
            return Err(Stop::Unavailable("program global receipt history is exhausted".into()));
        }
        program.globals.try_reserve_exact(1)
            .map_err(|_| Stop::Unavailable("program global receipt history could not reserve memory".into()))?;
        match program.registry.global_alloc(&mut self.memory, procedure, flags, bytes)? {
            program_modules::GlobalAllocReturn::OutOfMemory => Ok(0),
            program_modules::GlobalAllocReturn::Allocated(owner) => {
                // The real backend has just issued and validated this exact
                // receipt/storage under our exclusive borrows. No fallible
                // effect-acceptance or ledger reservation follows allocation.
                let pointer = program.registry.allocation_word(&self.memory, &owner)
                    .expect("fresh genuine registry receipt names its complete issuing Memory allocation");
                program.globals.push(GlobalReceipt { pointer, owner });
                Ok(pointer)
            }
        }
    }

    /// Only exact issued handles can select a retained receipt. Freed receipts
    /// stay bounded and permit the backend's genuine AlreadyReleased failure;
    /// unknown/interior/World words have no receipt and are host refusals.
    pub fn program_global_free(
        &mut self,
        access: &ProgramAccess,
        input: &Module,
        procedure_word: u32,
        handle: u32,
    ) -> Result<program_modules::GlobalFreeReturn, Stop> {
        self.require_program(access, input)?;
        let procedure = procedure(
            &self.capabilities, &self.program.as_ref().expect("authenticated program").registry,
            &self.memory, procedure_word, Export::GlobalFree,
        )?;
        let program = self.program.as_mut().expect("authenticated program");
        let receipt = program.globals.iter().find(|receipt| receipt.pointer == handle)
            .ok_or_else(|| unsupported("GlobalFree handle has no genuine receipt in this program"))?;
        program.registry.global_free(&mut self.memory, procedure, &receipt.owner, handle)
    }

    /// Consume and finish the backend's terminal transfer exactly once. Parent
    /// must propagate this typed termination out of its future program executor;
    /// no normal u32 return, host exit or outer Application cleanup is supplied.
    pub fn program_exit_process(
        &mut self,
        access: &ProgramAccess,
        input: &Module,
        procedure_word: u32,
        exit_code: u32,
    ) -> Result<ProgramTermination, Stop> {
        self.require_program(access, input)?;
        let procedure = procedure(
            &self.capabilities, &self.program.as_ref().expect("authenticated program").registry,
            &self.memory, procedure_word, Export::ExitProcess,
        )?;
        let program = self.program.as_mut().expect("authenticated program");
        let transfer = program.registry.exit_process(&mut self.memory, procedure, exit_code)?;
        let termination = program.registry.finish(&self.memory, transfer)
            .expect("exclusive issuing registry consumes its exact fresh terminal transfer once");
        program.sealed = true;
        program.globals.clear();
        self.capabilities.retain(|_, capability| !matches!(
            capability,
            Capability::ProgramModule(_) | Capability::ProgramImport(_) | Capability::ProgramProcedure(_)
                | Capability::ProgramRegistryKey(_)
                | Capability::ProgramRasterContext(_)
        ));
        Ok(termination)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
