//! Owning installation of genuine Rust-port logical import capabilities.
//!
//! Words belong to the retained BootstrapServices namespace, not to native or
//! Rust function addresses. All dependency/lease issuance and recoverable
//! reservations precede any IAT store. A refusal retains the exact input and
//! services, including monotonic partial host issuances, with zero image stores.
//! There is no retry, owner extraction, resolver callback or execution seam.

use super::{ImportPlan, MAX_DESCRIPTORS, MAX_IMPORTS};
use crate::native_bootstrap::{
    executable::{Module, ModuleOrigin, entry::{self, CurrentSpan, ProtectionCall}},
    services::{BootstrapServices, ProgramAccess, program_modules::{Configuration, ModuleIdentity, ModuleKind, ProcedureQuery}, program_ole32, program_raster, program_raster_context, program_registry_store, program_winmm},
};
use anyhow::{Context, Result, ensure};
use sims_formats::pe32::ImportSymbol;
#[cfg(test)]
use sims_sim::saved_matrix::Stop;
use std::{fmt, ops::Range};

/// The failed operation, in original descriptor/request order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStage {
    Preparation,
    BeginProgram,
    EnablePrivateRaster,
    Dependency(usize),
    Import(usize),
    ImageValidation,
}

/// A resolved descriptor, including zero-symbol descriptors. Repeated supported
/// descriptors share their exact deliberately loaded concrete module instance.
#[derive(Debug)]
pub struct Dependency {
    descriptor: usize,
    module_word: u32,
    identity: ModuleIdentity,
    loaded_here: bool,
}
impl Dependency {
    pub fn descriptor_index(&self) -> usize { self.descriptor }
    pub fn module_word(&self) -> u32 { self.module_word }
    pub fn identity(&self) -> &ModuleIdentity { &self.identity }
    pub fn kind(&self) -> ModuleKind { self.identity.kind() }
    pub fn loaded_here(&self) -> bool { self.loaded_here }
}

/// One successful program_lease_import issuance, even on a later refusal.
#[derive(Debug)]
pub struct ImportIssuance {
    request: usize,
    word: u32,
}
impl ImportIssuance {
    pub fn request_index(&self) -> usize { self.request }
    pub fn word(&self) -> u32 { self.word }
}

/// One actual IAT store. Initial Module bytes and source-store history are
/// separate observations. Neither this record nor its word grants authority.
#[derive(Debug)]
pub struct Write {
    address: u32,
    range: Range<usize>,
    before: [u8; 4],
    after: u32,
}
impl Write {
    pub fn address(&self) -> u32 { self.address }
    pub fn before(&self) -> [u8; 4] { self.before }
    pub fn before_word(&self) -> u32 { u32::from_le_bytes(self.before) }
    pub fn after(&self) -> u32 { self.after }
    pub(in crate::native_bootstrap::executable) fn range(&self) -> Range<usize> {
        self.range.clone()
    }
}

/// Immutable observations of the retained owners and this install attempt's
/// issuances/effects. Pre-existing service issuances remain in those services;
/// they are never claimed as this attempt's history. No mutable owner, access
/// token, image or extraction method is available.
pub struct InstallationState<'a> {
    plan: ImportPlan,
    services: BootstrapServices<'a>,
    access: Option<ProgramAccess>,
    // Retained on preparation/begin refusal, then owned by the registered program.
    pending_registry: Option<(program_registry_store::Hive, usize)>,
    // A refusal keeps the actual Device here or in the live program registry.
    pending_raster: Option<(program_raster::Device, program_raster_context::Policy, usize)>,
    dependencies: Vec<Dependency>,
    imports: Vec<ImportIssuance>,
    writes: Vec<Write>,
}
impl<'a> InstallationState<'a> {
    pub fn plan(&self) -> &ImportPlan { &self.plan }
    pub fn module(&self) -> &Module { self.plan.module() }
    pub fn services(&self) -> &BootstrapServices<'a> { &self.services }
    pub fn registered_by_install(&self) -> bool { self.access.is_some() }
    pub fn dependencies(&self) -> &[Dependency] { &self.dependencies }
    pub fn import_issuances(&self) -> &[ImportIssuance] { &self.imports }
    pub fn iat_writes(&self) -> &[Write] { &self.writes }
    pub fn source_writes(&self) -> &[entry::Write] { self.plan.entry.writes() }
    /// The actual retained Entry storage, including unmapped padding. Use
    /// current_span for mapped spans; this immutable slice grants no mapping
    /// or execution authority and cannot outlive or mutate the owning state.
    pub fn current_image(&self) -> &[u8] { self.plan.entry.current_image_bytes() }
    pub fn current_span(&self, address: u32, len: usize) -> Result<CurrentSpan<'_>> {
        self.plan.entry.current_span(address, len)
    }
}
impl fmt::Debug for InstallationState<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallationState")
            .field("plan", &self.plan)
            .field("registered_by_install", &self.registered_by_install())
            .field("dependencies", &self.dependencies)
            .field("import_issuances", &self.imports)
            .field("iat_writes", &self.writes).finish_non_exhaustive()
    }
}

/// Failure owns the exact consumed plan and services. It has no advance/cut,
/// reset/retry or into-parts API; dropping it drops its real leases together.
/// ```compile_fail
/// use sims_content::native_bootstrap::executable::imports::installed::InstallFailure;
/// fn advance_refusal(failure: &mut InstallFailure<'_>) { let _ = failure.advance(); }
/// ```
#[derive(Debug)]
pub struct InstallFailure<'a> {
    pub stage: InstallStage,
    pub error: anyhow::Error,
    state: InstallationState<'a>,
}
impl<'a> InstallFailure<'a> {
    pub fn state(&self) -> &InstallationState<'a> { &self.state }
}
impl fmt::Display for InstallFailure<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.stage, self.error)
    }
}
impl std::error::Error for InstallFailure<'_> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { Some(self.error.as_ref()) }
}

/// Non-Clone owner of the installed Entry, actual services and ProgramAccess.
/// Retained services hold the actual procedure leases. Only this owner can
/// advance its Entry, still stopping at the existing image-only driver cut.
///
/// The services cannot be mutated or separated through a borrowed observation:
/// ```compile_fail
/// use sims_content::native_bootstrap::executable::imports::installed::InstalledProgram;
/// fn separate(program: &InstalledProgram<'_>) {
///     let _ = program.state().services().memory_mut();
/// }
/// ```
/// ```compile_fail
/// use sims_content::native_bootstrap::executable::imports::installed::InstalledProgram;
/// fn duplicate<'a>(program: &InstalledProgram<'a>) -> InstalledProgram<'a> { program.clone() }
/// ```
/// A borrowed observation pins the owners while in use:
/// ```compile_fail
/// use sims_content::native_bootstrap::executable::imports::installed::InstalledProgram;
/// fn mutate_while_borrowed(program: &mut InstalledProgram<'_>) {
///     let bytes = program.state().current_image();
///     program.advance().unwrap();
///     let _ = bytes[0];
/// }
/// ```
#[derive(Debug)]
pub struct InstalledProgram<'a> {
    state: InstallationState<'a>,
}
impl<'a> InstalledProgram<'a> {
    pub fn state(&self) -> &InstallationState<'a> { &self.state }
    pub fn advance(&mut self) -> Result<entry::Cut> { self.state.plan.entry.advance() }
    pub fn pending_call(&self) -> Option<&ProtectionCall> { self.state.plan.entry.pending_call() }
    pub fn accepts_call(&self, call: &ProtectionCall) -> bool {
        self.state.plan.entry.accepts_call(call)
    }
}

// Construction and word assignment stay private to this module. The Entry
// sibling receives only a sealed, fully resolved batch; no caller can supply
// words or adopt an advanced Entry. Ranges/history are reserved before issuance.
pub(in crate::native_bootstrap::executable) struct ReadyImports {
    origin: ModuleOrigin,
    writes: Vec<Write>,
    terminators: Vec<(u32, Range<usize>)>,
}
impl ReadyImports {
    pub(in crate::native_bootstrap::executable) fn origin(&self) -> ModuleOrigin { self.origin.clone() }
    pub(in crate::native_bootstrap::executable) fn writes(&self) -> &[Write] { &self.writes }
    pub(in crate::native_bootstrap::executable) fn terminators(&self) -> &[(u32, Range<usize>)] {
        &self.terminators
    }
}

fn current_range(plan: &ImportPlan, address: u32, expected: [u8; 4]) -> Result<Range<usize>> {
    let span = plan.entry.current_span(address, 4)?;
    ensure!(span.bytes() == expected, "current import preimage or sentinel differs");
    let start = usize::try_from(address.checked_sub(plan.module().image().header().image_base)
        .context("import slot below preferred base")?)?;
    let end = start.checked_add(4).context("import slot extent overflows")?;
    ensure!(end <= plan.module().image().initial_bytes().len(), "import slot outside image");
    Ok(start..end)
}

fn prepare(state: &mut InstallationState<'_>) -> Result<ReadyImports> {
    let plan = &state.plan;
    ensure!(plan.len() <= MAX_IMPORTS && plan.descriptor_count() <= MAX_DESCRIPTORS,
        "import installation exceeds preflight bounds");
    ensure!(plan.entry.writes().is_empty() && plan.entry.pending_call().is_none(),
        "import plan must retain its unadvanced Entry");
    ensure!(plan.entry.current_image_bytes() == plan.module().image().initial_bytes(),
        "import plan current image is no longer initial");
    state.dependencies.try_reserve_exact(plan.descriptor_count()).context("reserve dependency history")?;
    state.imports.try_reserve_exact(plan.len()).context("reserve genuine import issuance history")?;
    let mut ready = ReadyImports { origin: plan.module().origin(), writes: Vec::new(), terminators: Vec::new() };
    ready.writes.try_reserve_exact(plan.len()).context("reserve exact IAT write history")?;
    ready.terminators.try_reserve_exact(plan.terminators.len()).context("reserve sentinel ranges")?;
    for slot in &plan.slots {
        let before = slot.initial_word.to_le_bytes();
        let range = current_range(plan, slot.address, before)?;
        ready.writes.push(Write { address: slot.address, range, before, after: 0 });
    }
    for &address in &plan.terminators {
        ready.terminators.push((address, current_range(plan, address, [0; 4])?));
    }
    Ok(ready)
}

fn resolve(state: &mut InstallationState<'_>, configuration: Configuration, ole32: program_ole32::Policy,
           winmm: program_winmm::Policy, ready: &mut ReadyImports,
           stage: &mut InstallStage) -> Result<()> {
    *stage = InstallStage::BeginProgram;
    state.access = Some(match state.pending_registry.take() {
        Some((hive, max_handles)) => match state.services.begin_program_with_private_user_registry(
            state.plan.module(), configuration, ole32, winmm, hive, max_handles,
        ) {
            Ok(access) => access,
            Err(failure) => {
                state.pending_registry = Some((failure.hive, max_handles));
                return Err(failure.error.into());
            }
        },
        None => state.services.begin_program_with_profiles(state.plan.module(), configuration, ole32, winmm)?,
    });
    let access = state.access.as_ref().expect("genuine begin_program succeeded");
    if let Some((device, policy, max_contexts)) = state.pending_raster.take() {
        *stage = InstallStage::EnablePrivateRaster;
        if let Err(failure) = state.services.program_enable_private_raster(
            access, state.plan.module(), device, policy, max_contexts,
        ) {
            state.pending_raster = Some((failure.device, policy, max_contexts));
            return Err(failure.error.into());
        }
    }
    let mut loaded = [(ModuleKind::Kernel32, None), (ModuleKind::Ole32, None),
        (ModuleKind::Winmm, None), (ModuleKind::Advapi32, None), (ModuleKind::Gdi32, None)];
    for (index, descriptor) in state.plan.descriptors().enumerate() {
        *stage = InstallStage::Dependency(index);
        let library = descriptor.library();
        // The real registry applies names and policy even with zero symbols.
        // Cache by sealed table kind, never by caller text or export spelling.
        let kind = state.services.program_dependency_kind(access, state.plan.module(), library)?;
        let cached = &mut loaded.iter_mut().find(|(candidate, _)| *candidate == kind)
            .expect("registry returned a sealed module kind").1;
        let loaded_here = cached.is_none();
        let module_word = match *cached {
            Some(word) => word,
            None => {
                let word = state.services.program_load_module(access, state.plan.module(), kind)?;
                *cached = Some(word);
                word
            }
        };
        let identity = state.services.program_module_identity(access, state.plan.module(), module_word)?;
        state.dependencies.push(Dependency { descriptor: index, module_word, identity, loaded_here });
        let info = &state.plan.descriptors[index];
        for request_index in info.first..info.first + info.count {
            *stage = InstallStage::Import(request_index);
            let request = state.plan.request(request_index).expect("preflight request order");
            let query = match request.symbol() {
                ImportSymbol::Name { name, .. } => ProcedureQuery::Name(name),
                ImportSymbol::Ordinal(ordinal) => ProcedureQuery::Ordinal(*ordinal),
            };
            let word = state.services.program_lease_import(access, state.plan.module(), request.library(), query)?;
            // These pushes use capacities reserved for the complete plan. The
            // word is assigned only from the actual owning service issuance.
            state.imports.push(ImportIssuance { request: request_index, word });
            ready.writes[request_index].after = word;
        }
    }
    Ok(())
}

impl ImportPlan {
    /// Consume the exact unadvanced plan and actual owning services. Failures
    /// preserve partial host issuance; every refused attempt has zero IAT and
    /// source effects. Existing service registration is never replaced.
    ///
    /// Bounds remain 4096 descriptors/requests, plus the services' monotonic
    /// shared 4096-capability budget and explicit registry limits. Recoverable
    /// Vec reservations precede issuance/stores; existing Arc/BTreeMap service
    /// metadata retains its host-OOM abort behavior.
    pub fn install<'a>(self, services: BootstrapServices<'a>, explicit_configuration: Configuration)
        -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>>
    {
        self.install_with_ole32(services, explicit_configuration, program_ole32::Policy::Disabled)
    }

    /// Additive owning OLE32 opt-in. InstalledProgram still provides no guest
    /// executor or mutable-services/extraction API. Parent's service dispatch
    /// must separately present a genuine retained apartment Return.
    pub fn install_with_ole32<'a>(self, services: BootstrapServices<'a>, explicit_configuration: Configuration,
                                ole32: program_ole32::Policy)
        -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>>
    {
        self.install_with_profiles(services, explicit_configuration, ole32, program_winmm::Policy::Disabled)
    }

    /// Explicit OLE32 and WINMM profiles for this owning install. Resolving
    /// timeGetTime retains an actual import lease without sampling a clock.
    /// All enabled sealed dependency kinds, including empty descriptors, share
    /// only their exact concrete registry instance within this attempt.
    pub fn install_with_profiles<'a>(self, services: BootstrapServices<'a>, explicit_configuration: Configuration,
                                   ole32: program_ole32::Policy, winmm: program_winmm::Policy)
        -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>>
    {
        self.install_profiles(services, explicit_configuration, ole32, winmm, None, None)
    }

    /// Explicit private logical current-user ASCII registry, backed by the
    /// supplied real hive. No settings or original caller arguments are guessed.
    /// Preparation/begin refusal retains the hive in InstallationState; later
    /// refusal retains it in the actual registered program. All image stores
    /// still require complete dependency and symbol resolution.
    pub fn install_with_private_user_registry<'a>(
        self, services: BootstrapServices<'a>, explicit_configuration: Configuration,
        ole32: program_ole32::Policy, winmm: program_winmm::Policy,
        hive: program_registry_store::Hive, max_handles: usize,
    ) -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>> {
        self.install_profiles(services, explicit_configuration, ole32, winmm, Some((hive, max_handles)), None)
    }

    /// Explicit software-raster-only opt-in. The real Device is retained on
    /// every refusal; enabling never invents a context or current-screen source.
    pub fn install_with_private_raster<'a>(
        self, services: BootstrapServices<'a>, explicit_configuration: Configuration,
        ole32: program_ole32::Policy, winmm: program_winmm::Policy,
        device: program_raster::Device, policy: program_raster_context::Policy, max_contexts: usize,
    ) -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>> {
        self.install_profiles(services, explicit_configuration, ole32, winmm,
            None, Some((device, policy, max_contexts)))
    }

    /// Explicit combined owning profile. Existing install APIs retain raster
    /// disabled. Dependencies and procedures still come only from the actual
    /// program registry; a later unsupported USER32 leaves all image bytes exact.
    pub fn install_with_private_user_registry_and_raster<'a>(
        self, services: BootstrapServices<'a>, explicit_configuration: Configuration,
        ole32: program_ole32::Policy, winmm: program_winmm::Policy,
        hive: program_registry_store::Hive, max_handles: usize,
        device: program_raster::Device, policy: program_raster_context::Policy, max_contexts: usize,
    ) -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>> {
        self.install_profiles(services, explicit_configuration, ole32, winmm,
            Some((hive, max_handles)), Some((device, policy, max_contexts)))
    }

    fn install_profiles<'a>(
        self, services: BootstrapServices<'a>, explicit_configuration: Configuration,
        ole32: program_ole32::Policy, winmm: program_winmm::Policy,
        pending_registry: Option<(program_registry_store::Hive, usize)>,
        pending_raster: Option<(program_raster::Device, program_raster_context::Policy, usize)>,
    ) -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>> {
        let mut state = InstallationState { plan: self, services, access: None, pending_registry, pending_raster,
            dependencies: Vec::new(), imports: Vec::new(), writes: Vec::new() };
        let mut stage = InstallStage::Preparation;
        let mut ready = match prepare(&mut state) {
            Ok(ready) => ready,
            Err(error) => return Err(InstallFailure { stage, error, state }),
        };
        if let Err(error) = resolve(&mut state, explicit_configuration, ole32, winmm, &mut ready, &mut stage) {
            return Err(InstallFailure { stage, error, state });
        }
        stage = InstallStage::ImageValidation;
        if let Err(error) = state.plan.entry.install_program_imports(&ready) {
            return Err(InstallFailure { stage, error, state });
        }
        // Moving pre-reserved, populated history and owners cannot fail or
        // allocate. The installed image is the Entry that advance will use.
        state.writes = ready.writes;
        Ok(InstalledProgram { state })
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
