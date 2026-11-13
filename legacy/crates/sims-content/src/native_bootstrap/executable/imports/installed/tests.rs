//! Meaningful authored controls, all UNRUN. Parent owns compilation/execution.
//! Fixtures use only authored temporary PE32 files and the real reader/services.

use super::*;
use crate::{Index, avatar_catalog::Directories, native_bootstrap::{Memory, LocalePolicy, executable::read}};
use crate::native_bootstrap::services::program_modules::{ExitPolicy, Kernel32Policy, Limits};
use std::{collections::BTreeMap, fs, io::Write as _, path::PathBuf,
    sync::atomic::{AtomicU64, Ordering}, time::{SystemTime, UNIX_EPOCH}};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

const BASE: u32 = 0x0040_0000;
const OPTIONAL: usize = 0x98;
const RECORDS: usize = 0x178;
const DATA_RAW: usize = 0x1a00;
const DATA_RVA: u32 = 0x1000;
const LOOKUP: u32 = 0x2002;
const IAT: u32 = 0x7000;
const EXPORTS: [&[u8]; 5] = [b"GetModuleHandleA", b"GetProcAddress", b"GlobalAlloc", b"GlobalFree", b"ExitProcess"];
static NEXT: AtomicU64 = AtomicU64::new(0);

// WINMM owning-installer controls, ALL UNRUN. Resolution and installation do
// no clock I/O. No installed/private executable is opened by these fixtures.
fn winmm_install<'a>(plan: ImportPlan, services: BootstrapServices<'a>, config: Configuration)
    -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>>
{
    plan.install_with_profiles(services, config, program_ole32::Policy::OwnedRustSta,
        program_winmm::Policy::DarwinKernelBootElapsed)
}
fn winmm_install_unsampled(state: &InstallationState<'_>) {
    let evidence = state.services().program_time_get_time_observation(
        state.access.as_ref().unwrap(), state.module()).unwrap();
    assert!(evidence.last_sample().is_none());
    assert!(evidence.latest_report().is_none());
    assert!(evidence.first_failure_report().is_none());
}

#[test]
fn installer_winmm_three_kinds_empty_and_repeated_descriptors_cache_actual_instances() {
    let timer = [Symbol::Name(b"timeGetTime")];
    let repeated = [Symbol::Name(b"timeGetTime"), Symbol::Name(b"timeGetTime")];
    let kernel = EXPORTS.map(Symbol::Name);
    let com = [Symbol::Name(b"CoUninitialize")];
    let case = Fixture::new(&fixture(&[
        (b"winmm", &[]), (b"kernel32", &[]), (b"ole32", &[]),
        (b"WINMM.DLL", &timer), (b"KERNEL32.DLL", &kernel), (b"OLE32.DLL", &com),
        (b"WiNmM", &repeated), (b"OlE32", &[]), (b"KeRnEl32", &[]),
    ], true));
    let plan = case.plan();
    let snapshot = Snapshot::new(&plan);
    let installed = winmm_install(plan, case.services(), configuration()).unwrap();
    let dependencies = installed.state().dependencies();
    assert_eq!(dependencies.len(), 9);
    for (index, kind) in [ModuleKind::Winmm, ModuleKind::Kernel32, ModuleKind::Ole32,
        ModuleKind::Winmm, ModuleKind::Kernel32, ModuleKind::Ole32,
        ModuleKind::Winmm, ModuleKind::Ole32, ModuleKind::Kernel32].into_iter().enumerate()
    {
        let first = match kind { ModuleKind::Winmm => 0, ModuleKind::Kernel32 => 1, ModuleKind::Ole32 => 2, ModuleKind::Advapi32 | ModuleKind::Gdi32 => unreachable!("fixture has no registry/raster descriptor") };
        assert_eq!(dependencies[index].descriptor_index(), index);
        assert_eq!(dependencies[index].kind(), kind);
        assert_eq!(dependencies[index].loaded_here(), index < 3);
        assert_eq!(dependencies[index].module_word(), dependencies[first].module_word());
        assert_eq!(dependencies[index].identity(), dependencies[first].identity());
    }
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert_ne!(dependencies[a].module_word(), dependencies[b].module_word());
        assert_ne!(dependencies[a].identity(), dependencies[b].identity());
    }
    assert_eq!(installed.state().import_issuances().len(), 9);
    assert_eq!(installed.state().iat_writes().len(), 9);
    assert_eq!(installed.state().services().live_capabilities(), 12);
    assert!(installed.state().source_writes().is_empty());
    snapshot.exact_effects(installed.state());
    winmm_install_unsampled(installed.state());
    let other = winmm_install(case.plan(), case.services(), configuration()).unwrap();
    for index in 0..3 {
        assert_eq!(dependencies[index].module_word(), other.state().dependencies()[index].module_word());
        assert_ne!(dependencies[index].identity(), other.state().dependencies()[index].identity());
    }
    winmm_install_unsampled(other.state());

    let empty = Fixture::new(&fixture(&[(b"WINMM", &[]), (b"OLE32", &[]), (b"KERNEL32", &[]),
        (b"winmm.dll", &[]), (b"ole32.dll", &[]), (b"kernel32.dll", &[])], false));
    let plan = empty.plan();
    let snapshot = Snapshot::new(&plan);
    let installed = winmm_install(plan, empty.services(), configuration()).unwrap();
    assert_eq!(installed.state().dependencies().len(), 6);
    assert!(installed.state().import_issuances().is_empty());
    assert!(installed.state().iat_writes().is_empty());
    assert_eq!(installed.state().services().live_capabilities(), 3);
    for index in 0..3 {
        assert!(installed.state().dependencies()[index].loaded_here());
        assert!(!installed.state().dependencies()[index + 3].loaded_here());
        assert_eq!(installed.state().dependencies()[index].identity(), installed.state().dependencies()[index + 3].identity());
    }
    snapshot.exact_effects(installed.state());
    winmm_install_unsampled(installed.state());
}

#[test]
fn installer_winmm_only_does_not_enable_kernel32_or_ole32_or_sample() {
    let timer = [Symbol::Name(b"timeGetTime")];
    let case = Fixture::new(&fixture(&[(b"WINMM.dll", &timer)], false));
    let mut config = configuration();
    config.kernel32 = Kernel32Policy::Disabled;
    let plan = case.plan();
    let snapshot = Snapshot::new(&plan);
    let installed = plan.install_with_profiles(case.services(), config, program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap();
    assert_eq!(installed.state().dependencies()[0].kind(), ModuleKind::Winmm);
    assert_eq!(installed.state().services().live_capabilities(), 2);
    let access = installed.state.access.as_ref().unwrap();
    for library in [b"kernel32".as_slice(), b"ole32"] {
        assert!(installed.state.services.program_dependency_kind(access, installed.state.module(), library).is_err());
    }
    snapshot.exact_effects(installed.state());
    winmm_install_unsampled(installed.state());
}

#[test]
fn installer_winmm_old_apis_and_explicit_disabled_refuse_even_empty_descriptors() {
    let timer = [Symbol::Name(b"timeGetTime")];
    for symbols in [timer.as_slice(), &[]] {
        let case = Fixture::new(&fixture(&[(b"winmm.dll", symbols)], false));
        for api in 0..3 {
            let plan = case.plan();
            let snapshot = Snapshot::new(&plan);
            let failure = match api {
                0 => plan.install(case.services(), configuration()),
                1 => plan.install_with_ole32(case.services(), configuration(), program_ole32::Policy::OwnedRustSta),
                _ => plan.install_with_profiles(case.services(), configuration(), program_ole32::Policy::OwnedRustSta,
                    program_winmm::Policy::Disabled),
            }.unwrap_err();
            assert_eq!(failure.stage, InstallStage::Dependency(0));
            assert!(failure.state().registered_by_install());
            assert!(failure.state().dependencies().is_empty());
            assert_eq!(failure.state().services().live_capabilities(), 0);
            snapshot.refused(&failure);
            winmm_install_unsampled(failure.state());
        }
    }
}

#[test]
fn installer_winmm_exact_names_ordinals_wrong_table_and_paths_keep_zero_stores() {
    for last in [Symbol::Name(b"TimeGetTime"), Symbol::Name(b"timegettime"),
        Symbol::Name(b"GlobalFree"), Symbol::Name(b"CoUninitialize"), Symbol::Ordinal(1)]
    {
        let symbols = [Symbol::Name(b"timeGetTime"), last];
        let case = Fixture::new(&fixture(&[(b"winmm.dll", &symbols)], false));
        let plan = case.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = winmm_install(plan, case.services(), configuration()).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Import(1));
        assert_eq!(failure.state().dependencies().len(), 1);
        assert_eq!(failure.state().import_issuances().len(), 1);
        assert_eq!(failure.state().services().live_capabilities(), 2);
        snapshot.refused(&failure);
        winmm_install_unsampled(failure.state());
    }
    for library in [b"kernel32".as_slice(), b"ole32"] {
        let case = Fixture::new(&fixture(&[(library, &[Symbol::Name(b"timeGetTime")])], false));
        let plan = case.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = winmm_install(plan, case.services(), configuration()).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Import(0));
        assert!(failure.state().import_issuances().is_empty());
        snapshot.refused(&failure);
        winmm_install_unsampled(failure.state());
    }
    for library in [b"dir/winmm.dll".as_slice(), b"C:\\winmm.dll", b"winmm.exe", b"ADVAPI32.dll"] {
        let case = Fixture::new(&fixture(&[(library, &[])], false));
        let plan = case.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = winmm_install(plan, case.services(), configuration()).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Dependency(0));
        assert_eq!(failure.state().services().live_capabilities(), 0);
        snapshot.refused(&failure);
        winmm_install_unsampled(failure.state());
    }
}

#[test]
fn installer_winmm_late_dependencies_and_budgets_preserve_partial_leases_no_effects() {
    let kernel = [Symbol::Name(b"GlobalFree")];
    let com = [Symbol::Name(b"CoUninitialize")];
    let timer = [Symbol::Name(b"timeGetTime")];
    let late = Fixture::new(&fixture(&[(b"kernel32", &kernel), (b"ole32", &com),
        (b"winmm", &timer), (b"ADVAPI32.dll", &[])], false));
    let plan = late.plan();
    let snapshot = Snapshot::new(&plan);
    let mut failure = winmm_install(plan, late.services(), configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(3));
    assert_eq!(failure.state().dependencies().len(), 3);
    assert_eq!(failure.state().import_issuances().len(), 3);
    assert_eq!(failure.state().services().live_capabilities(), 6);
    // Failure retains authenticated concrete dependency identity alongside the
    // partial import issuance history. Public observations cannot extract it.
    let state = &mut failure.state;
    let access = state.access.as_ref().unwrap();
    assert!(state.services.program_module_identity(access, state.plan.module(), state.dependencies[2].module_word()).is_ok());
    snapshot.refused(&failure);
    winmm_install_unsampled(failure.state());
    for module_limit in [true, false] {
        let case = Fixture::new(&fixture(&[(b"kernel32", &kernel), (b"ole32", &com), (b"winmm", &timer)], false));
        let mut config = configuration();
        if module_limit { config.limits.module_issuances = 2; }
        else { config.limits.procedure_lease_issuances = 2; }
        let plan = case.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = winmm_install(plan, case.services(), config).unwrap_err();
        assert_eq!(failure.stage, if module_limit { InstallStage::Dependency(2) } else { InstallStage::Import(2) });
        assert_eq!(failure.state().dependencies().len(), if module_limit { 2 } else { 3 });
        assert_eq!(failure.state().import_issuances().len(), 2);
        assert_eq!(failure.state().services().live_capabilities(), if module_limit { 4 } else { 5 });
        snapshot.refused(&failure);
        winmm_install_unsampled(failure.state());
    }
    // The same first unsupported dependency stays first with both profiles on.
    let first = Fixture::new(&fixture(&[(b"kernel32", &EXPORTS.map(Symbol::Name)),
        (b"ADVAPI32.dll", &[]), (b"winmm", &timer)], false));
    let plan = first.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = winmm_install(plan, first.services(), configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(1));
    assert_eq!(failure.state().import_issuances().len(), 5);
    assert_eq!(failure.state().services().live_capabilities(), 6);
    snapshot.refused(&failure);
    winmm_install_unsampled(failure.state());
}

#[test]
fn installer_winmm_cannot_replace_existing_program_or_reset_its_disabled_clock() {
    let timer = [Symbol::Name(b"timeGetTime")];
    let case = Fixture::new(&fixture(&[(b"winmm", &timer)], false));
    let plan = case.plan();
    let snapshot = Snapshot::new(&plan);
    let mut services = case.services();
    let original = services.begin_program(plan.module(), configuration()).unwrap();
    let failure = winmm_install(plan, services, configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::BeginProgram);
    assert!(!failure.state().registered_by_install());
    assert_eq!(failure.state().services().live_capabilities(), 0);
    let evidence = failure.state().services().program_time_get_time_observation(&original, failure.state().module()).unwrap();
    assert_eq!(evidence.policy(), program_winmm::Policy::Disabled);
    assert!(evidence.last_sample().is_none());
    assert!(evidence.latest_report().is_none());
    assert!(evidence.first_failure_report().is_none());
    snapshot.refused(&failure);
}

#[test]
#[ignore = "parent-only hash-pinned installed executable read with both profiles; authored UNRUN"]
fn installer_winmm_parent_installed_input_still_refuses_advapi32_dependency1() {
    let path = std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE")
        .expect("parent supplies fixed hash-verified installed executable");
    let module = read(std::path::Path::new(&path)).unwrap();
    assert_eq!(module.observation().bytes_read, 4_422_663);
    let digest: String = module.observation().sha256.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest, "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
    let plan = module.begin_initial_imports().unwrap();
    assert_eq!((plan.len(), plan.descriptor_count()), (13, 9));
    assert!(plan.descriptors().nth(1).unwrap().library().eq_ignore_ascii_case(b"ADVAPI32.dll"));
    let snapshot = Snapshot::new(&plan);
    let case = Fixture::new(&five());
    let failure = winmm_install(plan, case.services(), configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(1));
    assert_eq!(failure.state().dependencies().len(), 1);
    assert_eq!(failure.state().import_issuances().len(), 5);
    assert_eq!(failure.state().services().live_capabilities(), 6);
    snapshot.refused(&failure);
    winmm_install_unsampled(failure.state());
}

// Additive OLE32 controls, authored UNRUN. These use the real preflight,
// installer, sealed registry and service capabilities; no resolver is injected.
#[test]
fn installer_ole32_only_and_mixed_zero_symbol_dependencies_cache_exact_typed_instances() {
    let com = [Symbol::Name(b"CoUninitialize")];
    let only = Fixture::new(&fixture(&[(b"OLE32.dll", &com)], false));
    let mut ole_only = configuration();
    ole_only.kernel32 = Kernel32Policy::Disabled;
    let plan = only.plan();
    let snapshot = Snapshot::new(&plan);
    let installed = plan.install_with_ole32(only.services(), ole_only, program_ole32::Policy::OwnedRustSta).unwrap();
    assert_eq!(installed.state().dependencies()[0].kind(), ModuleKind::Ole32);
    assert_eq!(installed.state().import_issuances().len(), 1);
    assert_eq!(installed.state().services().live_capabilities(), 2);
    snapshot.exact_effects(installed.state());

    let mixed = Fixture::new(&fixture(&[
        (b"kernel32", &[]), (b"ole32.dll", &[]),
        (b"KERNEL32.DLL", &EXPORTS.map(Symbol::Name)), (b"OLE32", &com),
        (b"OlE32.DlL", &[]), (b"KeRnEl32", &[]),
    ], true));
    let plan = mixed.plan();
    let snapshot = Snapshot::new(&plan);
    let installed = plan.install_with_ole32(mixed.services(), configuration(), program_ole32::Policy::OwnedRustSta).unwrap();
    let dependencies = installed.state().dependencies();
    assert_eq!(dependencies.len(), 6);
    for (index, kind) in [ModuleKind::Kernel32, ModuleKind::Ole32, ModuleKind::Kernel32,
        ModuleKind::Ole32, ModuleKind::Ole32, ModuleKind::Kernel32].into_iter().enumerate() {
        assert_eq!(dependencies[index].descriptor_index(), index);
        assert_eq!(dependencies[index].kind(), kind);
        assert_eq!(dependencies[index].loaded_here(), index < 2);
        let first = if kind == ModuleKind::Kernel32 { 0 } else { 1 };
        assert_eq!(dependencies[index].module_word(), dependencies[first].module_word());
        assert_eq!(dependencies[index].identity(), dependencies[first].identity());
    }
    assert_ne!(dependencies[0].module_word(), dependencies[1].module_word());
    assert_ne!(dependencies[0].identity(), dependencies[1].identity());
    assert_eq!(installed.state().services().live_capabilities(), 8);
    assert_eq!(installed.state().iat_writes().len(), 6);
    assert!(installed.state().source_writes().is_empty());
    snapshot.exact_effects(installed.state());
}

#[test]
fn installer_ole32_disabled_unknown_wrong_table_ordinal_and_path_keep_zero_stores() {
    let com = [Symbol::Name(b"CoUninitialize")];
    // Both old install() and an explicit Disabled policy refuse even an empty
    // OLE32 descriptor before any module or procedure is issued.
    for symbols in [com.as_slice(), &[]] {
        let case = Fixture::new(&fixture(&[(b"ole32.dll", symbols)], false));
        for explicit in [false, true] {
            let plan = case.plan();
            let snapshot = Snapshot::new(&plan);
            let failure = if explicit {
                plan.install_with_ole32(case.services(), configuration(), program_ole32::Policy::Disabled)
            } else { plan.install(case.services(), configuration()) }.unwrap_err();
            assert_eq!(failure.stage, InstallStage::Dependency(0));
            assert_eq!(failure.state().services().live_capabilities(), 0);
            snapshot.refused(&failure);
        }
    }
    for (library, symbol) in [
        (b"kernel32".as_slice(), Symbol::Name(b"CoUninitialize")),
        (b"ole32", Symbol::Name(b"GlobalFree")),
        (b"ole32", Symbol::Name(b"CoInitialize")),
        (b"ole32", Symbol::Name(b"couninitialize")),
        (b"ole32", Symbol::Ordinal(1)),
    ] {
        let case = Fixture::new(&fixture(&[(library, &[symbol])], false));
        let plan = case.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = plan.install_with_ole32(case.services(), configuration(), program_ole32::Policy::OwnedRustSta).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Import(0));
        assert_eq!(failure.state().dependencies().len(), 1);
        assert!(failure.state().import_issuances().is_empty());
        assert_eq!(failure.state().services().live_capabilities(), 1);
        snapshot.refused(&failure);
    }
    for library in [b"dir/ole32.dll".as_slice(), b"C:\\ole32.dll", b"ole32.exe", b"ADVAPI32.dll"] {
        let case = Fixture::new(&fixture(&[(library, &[])], false));
        let plan = case.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = plan.install_with_ole32(case.services(), configuration(), program_ole32::Policy::OwnedRustSta).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Dependency(0));
        assert!(failure.state().dependencies().is_empty());
        assert_eq!(failure.state().services().live_capabilities(), 0);
        snapshot.refused(&failure);
    }
}

#[test]
fn installer_ole32_later_unknown_and_shared_issuance_limits_retain_partial_history_zero_stores() {
    let com = [Symbol::Name(b"CoUninitialize")];
    let kernel = EXPORTS.map(Symbol::Name);
    let late = Fixture::new(&fixture(&[(b"ole32", &com), (b"kernel32", &kernel), (b"ADVAPI32.dll", &[])], false));
    let plan = late.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install_with_ole32(late.services(), configuration(), program_ole32::Policy::OwnedRustSta).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(2));
    assert_eq!(failure.state().dependencies().len(), 2);
    assert_eq!(failure.state().import_issuances().len(), 6);
    assert_eq!(failure.state().services().live_capabilities(), 8);
    snapshot.refused(&failure);
    for module_limit in [true, false] {
        let case = Fixture::new(&fixture(&[(b"kernel32", &kernel), (b"ole32", &com)], false));
        let mut limits = configuration();
        if module_limit { limits.limits.module_issuances = 1; }
        else { limits.limits.procedure_lease_issuances = 5; }
        let plan = case.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = plan.install_with_ole32(case.services(), limits, program_ole32::Policy::OwnedRustSta).unwrap_err();
        assert_eq!(failure.stage, if module_limit { InstallStage::Dependency(1) } else { InstallStage::Import(5) });
        assert_eq!(failure.state().dependencies().len(), if module_limit { 1 } else { 2 });
        assert_eq!(failure.state().import_issuances().len(), 5);
        assert_eq!(failure.state().services().live_capabilities(), if module_limit { 6 } else { 7 });
        snapshot.refused(&failure);
    }
    // Source-authored shape of the installed first dependency: five K32
    // issuances survive an unsupported ADVAPI32 dependency1 with no stores.
    let dependency1 = Fixture::new(&fixture(&[(b"kernel32", &kernel), (b"ADVAPI32.dll", &[])], false));
    let plan = dependency1.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install_with_ole32(dependency1.services(), configuration(), program_ole32::Policy::OwnedRustSta).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(1));
    assert_eq!(failure.state().dependencies()[0].kind(), ModuleKind::Kernel32);
    assert_eq!(failure.state().import_issuances().len(), 5);
    snapshot.refused(&failure);
}

#[test]
fn installer_ole32_opt_in_cannot_replace_existing_disabled_program() {
    let com = [Symbol::Name(b"CoUninitialize")];
    let case = Fixture::new(&fixture(&[(b"ole32", &com)], false));
    let plan = case.plan();
    let snapshot = Snapshot::new(&plan);
    let mut services = case.services();
    let original = services.begin_program(plan.module(), configuration()).unwrap();
    let failure = plan.install_with_ole32(services, configuration(), program_ole32::Policy::OwnedRustSta).unwrap_err();
    assert_eq!(failure.stage, InstallStage::BeginProgram);
    assert!(!failure.state().registered_by_install());
    assert!(failure.state().services().program_dependency_kind(&original, failure.state().module(), b"ole32").is_err());
    snapshot.refused(&failure);
}

// Public Construction fixture for the owning installer-to-service path. The
// static images grant no COM authority: only the actual cursor/service call
// below creates and retains the successful Return.
fn installer_ole32_initialize(host: &mut BootstrapServices<'_>) -> (
    sims_sim::native_application::Construction,
    sims_sim::native_application::com::Request,
    sims_sim::native_application::com::Return,
) {
    use sims_sim::{native_application::{Call, Construction, Incoming}, native_world_children::Owner};
    let memory = host.memory_mut();
    let mut app = memory.allocate_owner(324).unwrap();
    app.data.fill(0x73);
    let mut argument = memory.allocate_owner(16).unwrap();
    argument.data[..5].copy_from_slice(b"Sims\0");
    let mut caller = memory.allocate_owner(96).unwrap();
    caller.data[2] = 0x7a;
    caller.data[6] = 0x59;
    caller.data[68..72].copy_from_slice(&(argument.token as u32).to_le_bytes());
    let mut nt_name = memory.allocate_owner(32).unwrap();
    let mut local_name = memory.allocate_owner(32).unwrap();
    nt_name.data[..12].copy_from_slice(b"Global\\Sims\0");
    local_name.data[..5].copy_from_slice(b"Sims\0");
    for owner in [&app, &argument, &caller, &nt_name, &local_name] { memory.synchronize_owner(owner).unwrap(); }
    let incoming = Incoming { application: app, argument: argument.token,
        tree_stack: [caller.token + 2, caller.token + 6], mutex_temporary: caller.token + 16,
        mutex_name_globals: Owner { token: 0x6c9f70,
            data: [nt_name.token as u32, local_name.token as u32].into_iter().flat_map(u32::to_le_bytes).collect() },
        mutex_singleton: Owner { token: 0x6f2a5c, data: 0x11223344_u32.to_le_bytes().to_vec() },
        retained: vec![argument, caller, nt_name, local_name], registry: Default::default(),
        registry_globals: Owner { token: 0x6de858, data: vec![0; 8] },
        application_singleton: Owner { token: 0x6f2a58, data: 0xaabbccdd_u32.to_le_bytes().to_vec() } };
    let mut construction = Construction::new(incoming).unwrap();
    let mut pending = None;
    for _ in 0..4096 {
        if let Some(Call::CoInitialize(request)) = construction.pending_call() { pending = Some(request); break; }
        construction.step(&mut host.application_host()).unwrap();
    }
    let request = pending.expect("Construction issues genuine COM request");
    for _ in 0..16 {
        if let Some(returned) = construction.com_initialization() {
            let returned = returned.clone();
            returned.validate(&request).unwrap();
            host.apartment(&returned).unwrap().check().unwrap();
            construction.validate().unwrap();
            return (construction, request, returned);
        }
        construction.step(&mut host.application_host()).unwrap();
    }
    panic!("Construction retains genuine COM return");
}

#[test]
fn installer_ole32_genuine_owning_lease_balances_exact_retained_s_false_return() {
    let com = [Symbol::Name(b"CoUninitialize")];
    let case = Fixture::new(&fixture(&[(b"ole32", &com)], false));
    let plan = case.plan();
    let initial = plan.module().image().initial_bytes().to_vec();
    let origin = plan.module().origin();
    let mut services = BootstrapServices::new(Memory::new(8 * 1024 * 1024).unwrap(),
        &case.index, &case.directories, LocalePolicy::C);
    let (first_history, first_request, first) = installer_ole32_initialize(&mut services);
    let (nested_history, nested_request, nested) = installer_ole32_initialize(&mut services);
    assert_eq!((first.hresult(), nested.hresult()), (0, 1));
    let live_nested = nested.current_apartment().unwrap();
    let before = (services.memory().live_bytes(), services.memory().live_allocations(), services.live_capabilities());
    let mut config = configuration();
    config.kernel32 = Kernel32Policy::Disabled;
    let installed = plan.install_with_ole32(services, config, program_ole32::Policy::OwnedRustSta).unwrap();
    assert_eq!(installed.state().module().origin(), origin);
    assert_eq!(installed.state().module().image().initial_bytes(), initial);
    assert!(installed.state().source_writes().is_empty());
    assert_eq!(installed.state().dependencies()[0].kind(), ModuleKind::Ole32);
    let word = installed.state().import_issuances()[0].word();
    assert_eq!(installed.state().iat_writes()[0].after(), word);
    let state = &installed.state;
    let access = state.access.as_ref().unwrap();
    let image = state.current_image().to_vec();
    // Unit-test visibility borrows the real sealed owners. No public owner
    // extraction, arbitrary executor, or mutable services seam is introduced.
    let (): () = state.services.program_co_uninitialize(access, state.plan.module(), word, &nested).unwrap();
    assert!(live_nested.check().is_err());
    assert!(state.services.apartment(&nested).is_err());
    assert_eq!(state.services.apartment(&first).unwrap().initialization_count(), 1);
    assert!(state.services.program_co_uninitialize(access, state.plan.module(), word, &nested).is_err());
    assert_eq!(state.current_image(), image);
    assert_eq!((state.services.memory().live_bytes(), state.services.memory().live_allocations(),
        state.services.live_capabilities()), (before.0, before.1, before.2 + 2));
    nested.validate(&nested_request).unwrap();
    nested_history.validate().unwrap();
    state.services.program_co_uninitialize(access, state.plan.module(), word, &first).unwrap();
    first.validate(&first_request).unwrap();
    first_history.validate().unwrap();
    assert_eq!(state.current_image(), image);
}

#[test]
#[ignore = "explicit fixed installed executable read only; no native execution"]
fn installer_parent_installed_thirteen_imports_refuse_without_image_effects() {
    let path = std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE")
        .expect("parent must supply the fixed hash-verified installed executable");
    let module = read(std::path::Path::new(&path)).unwrap();
    assert_eq!(module.observation().bytes_read, 4_422_663);
    let digest: String = module.observation().sha256.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest, "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
    let plan = module.begin_initial_imports().unwrap();
    assert_eq!((plan.len(), plan.descriptor_count()), (13, 9));
    let snapshot = Snapshot::new(&plan);
    let fixture = Fixture::new(&five());
    let failure = plan.install(fixture.services(), configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(1));
    snapshot.refused(&failure);
    assert!(failure.state().registered_by_install());
    assert_eq!(failure.state().dependencies().len(), 1);
    assert_eq!(failure.state().import_issuances().len(), 5);
    assert_eq!(failure.state().services().live_capabilities(), 6);
}

#[test]
#[ignore = "parent-only fixed installed executable read; authored UNRUN, no native execution"]
fn installer_ole32_parent_installed_input_still_refuses_advapi32_dependency1() {
    let path = std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE")
        .expect("parent supplies fixed hash-verified installed executable");
    let module = read(std::path::Path::new(&path)).unwrap();
    assert_eq!(module.observation().bytes_read, 4_422_663);
    let digest: String = module.observation().sha256.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest, "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
    let plan = module.begin_initial_imports().unwrap();
    assert_eq!((plan.len(), plan.descriptor_count()), (13, 9));
    assert!(plan.descriptors().nth(1).unwrap().library().eq_ignore_ascii_case(b"ADVAPI32.dll"));
    let snapshot = Snapshot::new(&plan);
    let case = Fixture::new(&five());
    let failure = plan.install_with_ole32(case.services(), configuration(), program_ole32::Policy::OwnedRustSta).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(1));
    assert_eq!(failure.state().dependencies().len(), 1);
    assert_eq!(failure.state().dependencies()[0].kind(), ModuleKind::Kernel32);
    assert_eq!(failure.state().import_issuances().len(), 5);
    assert_eq!(failure.state().services().live_capabilities(), 6);
    snapshot.refused(&failure);
}

struct Fixture { directory: PathBuf, path: PathBuf, index: Index, directories: Directories }
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let directory = loop {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("sims-import-install-{nanos}-{sequence}"));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create authored fixture: {error}"),
            }
        };
        let path = directory.join("authored.exe");
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o644).open(&path).unwrap();
        file.write_all(bytes).unwrap();
        let index = Index { root: directory.clone(), resources: vec![], archives: 0, diagnostics: vec![] };
        Self { directory, path, index, directories: Directories::default() }
    }
    fn plan(&self) -> ImportPlan { read(&self.path).unwrap().begin_initial_imports().unwrap() }
    fn services(&self) -> BootstrapServices<'_> {
        BootstrapServices::new(Memory::new(8192).unwrap(), &self.index, &self.directories, LocalePolicy::C)
    }
}
impl Drop for Fixture { fn drop(&mut self) { fs::remove_dir_all(&self.directory).unwrap(); } }

fn configuration() -> Configuration {
    Configuration { kernel32: Kernel32Policy::RustPort,
        exit: ExitPolicy::SingleThreadNoGuestDllDetachCallbacks,
        limits: Limits { module_issuances: 4, procedure_lease_issuances: 4096,
            weak_observation_issuances: 64, global_allocation_issuances: 16,
            max_live_global_bytes: 4096, max_global_bytes: 1024, max_name_bytes: 64 } }
}

fn put16(bytes: &mut [u8], at: usize, value: u16) { bytes[at..at+2].copy_from_slice(&value.to_le_bytes()); }
fn put32(bytes: &mut [u8], at: usize, value: u32) { bytes[at..at+4].copy_from_slice(&value.to_le_bytes()); }
fn raw(rva: u32) -> usize { DATA_RAW + (rva - DATA_RVA) as usize }
fn word(bytes: &mut [u8], rva: u32, value: u32) { put32(bytes, raw(rva), value); }
fn string(bytes: &mut [u8], rva: u32, value: &[u8]) {
    let at = raw(rva);bytes[at..at+value.len()].copy_from_slice(value);
}
fn directory(bytes: &mut [u8], index: usize, rva: u32, size: u32) {
    put32(bytes, OPTIONAL + 96 + index*8, rva);
    put32(bytes, OPTIONAL + 100 + index*8, size);
}
fn descriptor(bytes: &mut [u8], at: u32, lookup: u32, library: u32, iat: u32) {
    for (index,value) in [lookup,0,u32::MAX,library,iat].into_iter().enumerate() {
        word(bytes, at+index as u32*4, value);
    }
}
fn section(bytes: &mut [u8], index: usize, name: &[u8;8], rva: u32,
           virtual_size: u32, raw_size: u32, raw_at: u32, flags: u32) {
    let at = RECORDS + index*40;
    bytes[at..at+40].fill(0);bytes[at..at+8].copy_from_slice(name);
    for (offset,value) in [(8,virtual_size),(12,rva),(16,raw_size),(20,raw_at),(36,flags)] {
        put32(bytes,at+offset,value);
    }
}

// Authored PE32 with sorted .idata/.text/stxt371 spans. Only the entry bytes
// needed by the existing owning Entry profile are actual source observations;
// unrelated filler/data is authored, not protected code, CRT or execution.
fn base_pe() -> Vec<u8> {
    let mut bytes = vec![0; DATA_RAW+0x10000];
    bytes[..2].copy_from_slice(b"MZ");put32(&mut bytes,0x3c,0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes,0x84,0x014c);put16(&mut bytes,0x86,3);
    put16(&mut bytes,0x94,224);put16(&mut bytes,0x96,0x0102);
    put16(&mut bytes,OPTIONAL,0x010b);
    for (offset,value) in [(4,0x1800),(12,0x10000),(16,0x31406e),(20,0x238000),
        (24,0x1000),(28,BASE),(32,0x1000),(36,0x200),(56,0x315000),(60,0x200),
        (72,0x100000),(76,0x1000),(80,0x100000),(84,0x1000),(92,16)] {
        put32(&mut bytes,OPTIONAL+offset,value);
    }
    put16(&mut bytes,OPTIONAL+40,4);put16(&mut bytes,OPTIONAL+48,4);
    put16(&mut bytes,OPTIONAL+68,3);
    section(&mut bytes,0,b".idata\0\0",DATA_RVA,0x10000,0x10000,DATA_RAW as u32,0xc0000040);
    section(&mut bytes,1,b".text\0\0\0",0x238000,0x800,0x600,0x200,0x60000020);
    section(&mut bytes,2,b"stxt371\0",0x313000,0x1800,0x1200,0x800,0xe0000020);
    for (index,byte) in bytes[0x200..0x1a00].iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(29).wrapping_add(0x53);
    }
    let source = concat!(
        "558bec60bb6e407100b80d40710033c98a0885c9740cb8e44071002bc383e805eb0e",
        "51b92b4171008bc12bc303410159c603e98943015168d93f710033c085c974058b4508",
        "eb0050e825fcffff");
    let source: Vec<u8> = source.as_bytes().chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect();
    let entry = 0x800+(0x31406e-0x313000);
    bytes[entry..entry+77].copy_from_slice(&source);
    bytes[0x800+(0x31400d-0x313000)] = 0;
    let jump = 0x800+(0x31412b-0x313000);
    bytes[jump..jump+5].copy_from_slice(&[0xe9,0x34,0x42,0xf2,0xff]);
    bytes
}

#[derive(Clone, Copy)]
enum Symbol<'a> { Name(&'a [u8]), Ordinal(u16) }

fn fixture(descriptors: &[(&[u8], &[Symbol<'_>])], unaligned: bool) -> Vec<u8> {
    let mut bytes = base_pe();
    let mut lookup = LOOKUP;
    let first_iat = IAT + u32::from(unaligned);
    let mut iat = first_iat;
    let mut library_rva = 0x1800;
    let mut name_rva = 0x1400;
    let mut names = BTreeMap::<Vec<u8>, u32>::new();
    for (index, (library, symbols)) in descriptors.iter().enumerate() {
        descriptor(&mut bytes, 0x1200 + index as u32 * 20, lookup, library_rva, iat);
        string(&mut bytes, library_rva, library);
        library_rva += library.len() as u32 + 1;
        for symbol in *symbols {
            let value = match symbol {
                Symbol::Ordinal(ordinal) => 0x8000_0000 | u32::from(*ordinal),
                Symbol::Name(name) => match names.get(*name) {
                    Some(rva) => *rva,
                    None => {
                        let rva = name_rva;
                        put16(&mut bytes, raw(rva), 7);
                        string(&mut bytes, rva + 2, name);
                        name_rva += name.len() as u32 + 3;
                        names.insert(name.to_vec(), rva);
                        rva
                    }
                },
            };
            word(&mut bytes, lookup, value);
            word(&mut bytes, iat, value);
            lookup += 4;
            iat += 4;
        }
        // Both tables' actual terminators are already zero in authored data.
        lookup += 4;
        iat += 4;
    }
    assert!(name_rva < 0x1800 && library_rva < LOOKUP && lookup <= first_iat);
    assert!(iat <= DATA_RVA + 0x10000);
    if !descriptors.is_empty() {
        directory(&mut bytes, 1, 0x1200, (descriptors.len() as u32 + 1) * 20);
        directory(&mut bytes, 12, first_iat, iat - first_iat);
    }
    bytes
}

fn five() -> Vec<u8> { fixture(&[(b"KERNEL32.dll", &EXPORTS.map(Symbol::Name))], false) }

struct Snapshot {
    origin: ModuleOrigin,
    observation: crate::native_bootstrap::executable::Observation,
    raw: Vec<u8>,
    initial: Vec<u8>,
    raw_pointer: *const u8,
    initial_pointer: *const u8,
    current_pointer: *const u8,
}
impl Snapshot {
    fn new(plan: &ImportPlan) -> Self {
        let image = plan.module().image();
        Self { origin: plan.module().origin(), observation: plan.module().observation().clone(),
            raw: image.raw().to_vec(), initial: image.initial_bytes().to_vec(),
            raw_pointer: image.raw().as_ptr(), initial_pointer: image.initial_bytes().as_ptr(),
            current_pointer: plan.entry.current_image_bytes().as_ptr() }
    }
    fn immutable(&self, state: &InstallationState<'_>) {
        assert_eq!(state.module().origin(), self.origin);
        assert_eq!(state.module().observation(), &self.observation);
        assert_eq!(state.module().image().raw(), self.raw);
        assert_eq!(state.module().image().initial_bytes(), self.initial);
        assert_eq!(state.module().image().raw().as_ptr(), self.raw_pointer);
        assert_eq!(state.module().image().initial_bytes().as_ptr(), self.initial_pointer);
        assert_eq!(state.current_image().as_ptr(), self.current_pointer);
        assert_eq!(state.services().memory().live_allocations(), 0);
        assert_eq!(state.services().memory().live_bytes(), 0);
        for address in state.plan().terminators() {
            assert_eq!(state.current_span(*address, 4).unwrap().bytes(), [0; 4]);
            assert_eq!(state.module().initial_span(*address, 4).unwrap().bytes(), [0; 4]);
        }
    }
    fn refused(&self, failure: &InstallFailure<'_>) {
        self.immutable(failure.state());
        assert_eq!(failure.state().current_image(), self.initial);
        assert!(failure.state().iat_writes().is_empty());
        assert!(failure.state().source_writes().is_empty());
        assert!(failure.state.plan.entry.pending_call().is_none());
    }
    fn exact_effects(&self, state: &InstallationState<'_>) {
        self.immutable(state);
        let mut expected = self.initial.clone();
        for write in state.iat_writes() {
            let start = (write.address() - BASE) as usize;
            assert_eq!(write.before(), self.initial[start..start + 4]);
            expected[start..start + 4].copy_from_slice(&write.after().to_le_bytes());
        }
        for write in state.source_writes() {
            let start = (write.address() - BASE) as usize;
            assert_eq!(write.before(), &self.initial[start..start + write.before().len()]);
            expected[start..start + write.after().len()].copy_from_slice(write.after());
        }
        // Compare the actual complete Entry storage (including gaps/padding),
        // rather than reconstructing a detached image from mapped-span views.
        assert_eq!(state.current_image(), expected);
        let actual_diff: Vec<_> = state.current_image().iter().zip(&self.initial).enumerate()
            .filter_map(|(index, (after, before))| (after != before).then_some((index, *before, *after))).collect();
        let expected_diff: Vec<_> = expected.iter().zip(&self.initial).enumerate()
            .filter_map(|(index, (after, before))| (after != before).then_some((index, *before, *after))).collect();
        assert_eq!(actual_diff, expected_diff);
    }
}

#[test]
fn installer_five_exports_genuine_namespace_exact_image_and_repeat_cut() {
    let bytes = five();
    let fixture = Fixture::new(&bytes);
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let mut installed = plan.install(fixture.services(), configuration()).unwrap();
    assert!(installed.state().registered_by_install());
    assert_eq!(installed.state().dependencies().len(), 1);
    let dependency = &installed.state().dependencies()[0];
    assert!(dependency.loaded_here());
    assert_eq!(dependency.descriptor_index(), 0);
    assert_eq!(installed.state().import_issuances().len(), 5);
    assert_eq!(installed.state().services().live_capabilities(), 6);
    assert_eq!(installed.state().iat_writes().len(), 5);
    assert!(installed.pending_call().is_none());
    assert!(installed.state().source_writes().is_empty());
    let module_word = dependency.module_word();
    for (index, issuance) in installed.state().import_issuances().iter().enumerate() {
        assert_eq!(issuance.request_index(), index);
        assert_ne!(issuance.word(), 0);
        assert_ne!(issuance.word(), module_word);
        assert!(installed.state().import_issuances()[..index].iter().all(|other| other.word() != issuance.word()));
        let request = installed.state().plan().request(index).unwrap();
        assert_eq!(request.library(), b"KERNEL32.dll");
        assert_eq!(request.symbol(), &ImportSymbol::Name { hint: 7, name: EXPORTS[index].to_vec() });
        let write = &installed.state().iat_writes()[index];
        assert_eq!(write.address(), request.slot_address());
        assert_eq!(write.before(), request.initial_word().to_le_bytes());
        assert_eq!(write.before_word(), request.initial_word());
        assert_eq!(write.after(), issuance.word());
    }
    snapshot.exact_effects(installed.state());

    // Actual typed service dispatch verifies module/procedure roles and the
    // concrete exports using the retained issuer/access, with no guest call.
    // Installation creates no weak-procedure observations: every procedure
    // word above comes from the concrete program_lease_import operation.
    let state = &mut installed.state;
    let access = state.access.as_ref().unwrap();
    let get_module = state.imports[0].word();
    let get_proc = state.imports[1].word();
    assert_eq!(state.services.program_get_module_handle_a(access, state.plan.module(), get_module,
        Some(b"absent.dll")).unwrap(), 0);
    assert_eq!(state.services.program_get_proc_address(access, state.plan.module(), get_proc,
        module_word, ProcedureQuery::Name(b"AbsentExport")).unwrap(), 0);
    assert!(state.services.program_get_proc_address(access, state.plan.module(), module_word,
        module_word, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert!(state.services.program_get_proc_address(access, state.plan.module(), get_proc,
        get_module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    for issuance in &state.imports[1..] {
        let error = state.services.program_get_module_handle_a(access, state.plan.module(), issuance.word(),
            Some(b"absent.dll")).unwrap_err();
        assert!(error.to_string().contains("another concrete export"));
    }
    assert_eq!(state.services.live_capabilities(), 6);
    assert_eq!(installed.advance().unwrap(), entry::Cut::BeforeProtectionDriver);
    let call = installed.pending_call().unwrap().clone();
    assert_eq!(call.origin(), snapshot.origin);
    assert_eq!((call.instruction(), call.target(), call.arguments()), (0x7140b6, 0x713ce0, [0, 0x713fd9]));
    assert!(installed.accepts_call(&call));
    assert_eq!(installed.state().source_writes().len(), 2);
    assert_eq!(installed.state().source_writes()[0].address(), 0x71406e);
    assert_eq!(installed.state().source_writes()[0].after(), [0xe9]);
    assert_eq!(installed.state().source_writes()[1].address(), 0x71406f);
    assert_eq!(installed.state().source_writes()[1].after(), 0xfff2_42f1u32.to_le_bytes());
    snapshot.exact_effects(installed.state());
    let once = installed.state().current_image().to_vec();
    assert_eq!(installed.advance().unwrap(), entry::Cut::BeforeProtectionDriver);
    assert!(installed.pending_call().unwrap().same_call(&call));
    assert!(installed.accepts_call(&call));
    assert_eq!(installed.state().source_writes().len(), 2);
    assert_eq!(installed.state().services().live_capabilities(), 6);
    assert_eq!(installed.state().current_image(), once);
    snapshot.exact_effects(installed.state());
}

#[test]
fn installer_zero_imports_registers_without_loading_or_writing() {
    let fixture = Fixture::new(&fixture(&[], false));
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let mut installed = plan.install(fixture.services(), configuration()).unwrap();
    assert!(installed.state().registered_by_install());
    assert!(installed.state().plan().is_empty());
    assert!(installed.state().dependencies().is_empty());
    assert!(installed.state().import_issuances().is_empty());
    assert!(installed.state().iat_writes().is_empty());
    assert_eq!(installed.state().services().live_capabilities(), 0);
    snapshot.exact_effects(installed.state());
    installed.advance().unwrap();
    assert_eq!(installed.state().source_writes().len(), 2);
    snapshot.exact_effects(installed.state());
}

#[test]
fn installer_keeps_existing_service_namespace_and_unrelated_file_capability() {
    use sims_sim::native_world::resources::path;
    let mut fixture = Fixture::new(&five());
    fixture.directories.insert_first(b"./GameData/", b"Scenario.tds", 0).unwrap();
    fixture.index.resources.push(crate::Resource {
        locator: "./GameData/Scenario.tds".into(), name: "Scenario.tds".into(),
        source: fixture.directory.join("Scenario.tds"), offset: 0, size: 11,
        unpacked_size: 11, archived: false, owned: Some(b"world bytes".to_vec().into()),
    });
    let mut services = fixture.services();
    let opened = path::Host::open_scenario(&mut services, b"./GameData/Scenario.tds", b"rb").unwrap();
    let path::Opened::File(file_word) = opened else { panic!("authored mounted file must resolve") };
    let file_word = u32::try_from(file_word).unwrap();
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let installed = plan.install(services, configuration()).unwrap();
    assert_eq!(installed.state().services().live_capabilities(), 7);
    assert_eq!(installed.state().services().file(file_word).unwrap().bytes, b"world bytes");
    assert_eq!(installed.state().dependencies()[0].module_word(), file_word + 1);
    assert!(installed.state().services().file(installed.state().dependencies()[0].module_word()).is_err());
    for (index, issuance) in installed.state().import_issuances().iter().enumerate() {
        assert_eq!(issuance.word(), file_word + 2 + index as u32);
        assert!(installed.state().services().file(issuance.word()).is_err());
    }
    snapshot.exact_effects(installed.state());
}

#[test]
fn installer_empty_dependencies_are_resolved_and_unknown_ones_refuse() {
    let known = Fixture::new(&fixture(&[(b"KeRnEl32", &[])], false));
    let plan = known.plan();
    let snapshot = Snapshot::new(&plan);
    let installed = plan.install(known.services(), configuration()).unwrap();
    assert_eq!(installed.state().dependencies().len(), 1);
    assert_eq!(installed.state().services().live_capabilities(), 1);
    assert!(installed.state().iat_writes().is_empty());
    snapshot.exact_effects(installed.state());

    let unknown = Fixture::new(&fixture(&[(b"EMPTY.dll", &[])], false));
    let plan = unknown.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install(unknown.services(), configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(0));
    assert!(failure.state().registered_by_install());
    assert!(failure.state().dependencies().is_empty());
    assert!(failure.state().import_issuances().is_empty());
    assert_eq!(failure.state().services().live_capabilities(), 0);
    snapshot.refused(&failure);
}

#[test]
fn installer_unaligned_slots_and_duplicate_descriptors_keep_original_order() {
    let one = [Symbol::Name(b"GlobalFree"), Symbol::Name(b"GlobalFree")];
    let two = [Symbol::Name(b"GetProcAddress")];
    let fixture = Fixture::new(&fixture(&[(b"kernel32", &one), (b"KERNEL32.DLL", &two)], true));
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    assert!(plan.requests().all(|request| request.slot_address() % 4 == 1));
    let installed = plan.install(fixture.services(), configuration()).unwrap();
    let dependencies = installed.state().dependencies();
    assert_eq!(dependencies.len(), 2);
    assert!(dependencies[0].loaded_here());
    assert!(!dependencies[1].loaded_here());
    assert_eq!(dependencies[0].module_word(), dependencies[1].module_word());
    assert_eq!(dependencies[1].descriptor_index(), 1);
    assert_eq!(installed.state().services().live_capabilities(), 4);
    assert_eq!(installed.state().iat_writes().len(), 3);
    for (index, write) in installed.state().iat_writes().iter().enumerate() {
        assert_eq!(installed.state().import_issuances()[index].request_index(), index);
        assert_eq!(write.address(), installed.state().plan().request(index).unwrap().slot_address());
    }
    assert_eq!(installed.state().plan().request(2).unwrap().descriptor_index(), 1);
    snapshot.exact_effects(installed.state());
}

#[test]
fn installer_late_symbol_and_ordinal_refusals_retain_partial_issuance() {
    for last in [Symbol::Name(b"MissingExport"), Symbol::Name(b"globalalloc"), Symbol::Ordinal(42)] {
        let symbols = [Symbol::Name(b"GlobalAlloc"), Symbol::Name(b"GlobalFree"), last];
        let fixture = Fixture::new(&fixture(&[(b"KERNEL32.dll", &symbols)], false));
        let plan = fixture.plan();
        let snapshot = Snapshot::new(&plan);
        let failure = plan.install(fixture.services(), configuration()).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Import(2));
        assert!(failure.state().registered_by_install());
        assert_eq!(failure.state().dependencies().len(), 1);
        assert_eq!(failure.state().import_issuances().len(), 2);
        assert_eq!(failure.state().services().live_capabilities(), 3);
        assert!(failure.error.downcast_ref::<Stop>().is_some());
        snapshot.refused(&failure);
    }
}

#[test]
fn installer_late_unknown_empty_dll_never_commits_earlier_resolved_slots() {
    let symbols = [Symbol::Name(b"GlobalAlloc")];
    let fixture = Fixture::new(&fixture(&[(b"kernel32.dll", &symbols), (b"USER32.dll", &[])], false));
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install(fixture.services(), configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(1));
    assert_eq!(failure.state().dependencies().len(), 1);
    assert_eq!(failure.state().import_issuances().len(), 1);
    assert_eq!(failure.state().services().live_capabilities(), 2);
    snapshot.refused(&failure);
}

#[test]
fn installer_real_registry_and_shared_capability_budgets_have_zero_image_effects() {
    let symbols = EXPORTS.map(Symbol::Name);
    let fixture_small = Fixture::new(&fixture(&[(b"kernel32", &symbols)], false));
    let plan = fixture_small.plan();
    let snapshot = Snapshot::new(&plan);
    let mut small = configuration();
    small.limits.procedure_lease_issuances = 2;
    let failure = plan.install(fixture_small.services(), small).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Import(2));
    assert_eq!(failure.state().import_issuances().len(), 2);
    assert_eq!(failure.state().services().live_capabilities(), 3);
    assert!(failure.error.to_string().contains("program import procedure history is exhausted"));
    snapshot.refused(&failure);

    // Real issuances fill the unmodified service's 4096 shared-capability
    // budget. No private counter edit or fabricated backend/token is used.
    let repeated = vec![Symbol::Name(b"GlobalAlloc"); MAX_IMPORTS];
    let fixture_full = Fixture::new(&fixture(&[(b"kernel32", &repeated)], false));
    let plan = fixture_full.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install(fixture_full.services(), configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Import(4095));
    assert_eq!(failure.state().import_issuances().len(), 4095);
    assert_eq!(failure.state().dependencies().len(), 1);
    assert_eq!(failure.state().services().live_capabilities(), 4096);
    assert!(failure.error.to_string().contains("bootstrap capability profile exhausted"));
    snapshot.refused(&failure);
}

#[test]
fn installer_invalid_and_disabled_configuration_preserve_exact_owners() {
    let fixture = Fixture::new(&five());
    let mut invalid = configuration();
    invalid.limits.procedure_lease_issuances = 0;
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install(fixture.services(), invalid).unwrap_err();
    assert_eq!(failure.stage, InstallStage::BeginProgram);
    assert!(!failure.state().registered_by_install());
    assert_eq!(failure.state().services().live_capabilities(), 0);
    snapshot.refused(&failure);
    let mut disabled = configuration();
    disabled.kernel32 = Kernel32Policy::Disabled;
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install(fixture.services(), disabled).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(0));
    assert!(failure.state().registered_by_install());
    assert_eq!(failure.state().services().live_capabilities(), 0);
    snapshot.refused(&failure);

    let mut no_exit = configuration();
    no_exit.exit = ExitPolicy::Disabled;
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install(fixture.services(), no_exit).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Import(4));
    assert_eq!(failure.state().import_issuances().len(), 4);
    snapshot.refused(&failure);
}

#[test]
fn installer_existing_registration_is_retained_and_never_replaced() {
    let fixture = Fixture::new(&five());
    let plan = fixture.plan();
    let snapshot = Snapshot::new(&plan);
    let mut services = fixture.services();
    let old_access = services.begin_program(plan.module(), configuration()).unwrap();
    let old_module = services.program_load_kernel32(&old_access, plan.module()).unwrap();
    let old_proc = services.program_lease_import(&old_access, plan.module(), b"kernel32",
        ProcedureQuery::Name(b"GetProcAddress")).unwrap();
    let mut failure = plan.install(services, configuration()).unwrap_err();
    assert_eq!(failure.stage, InstallStage::BeginProgram);
    assert!(!failure.state().registered_by_install());
    assert!(failure.state().dependencies().is_empty());
    assert!(failure.state().import_issuances().is_empty());
    assert_eq!(failure.state().services().live_capabilities(), 2);
    snapshot.refused(&failure);
    // Unit-test-only private access proves the prior registration is still
    // live. No public failure retry or mutable-service API is introduced.
    let state = &mut failure.state;
    assert_eq!(state.services.program_get_proc_address(&old_access, state.plan.module(), old_proc,
        old_module, ProcedureQuery::Name(b"MissingExport")).unwrap(), 0);
    assert!(state.services.begin_program(state.plan.module(), configuration()).is_err());
    snapshot.refused(&failure);
}

#[test]
fn installer_same_bytes_foreign_services_and_dropped_calls_remain_distinct() {
    let fixture = Fixture::new(&five());
    let mut first = fixture.plan().install(fixture.services(), configuration()).unwrap();
    let mut second = fixture.plan().install(fixture.services(), configuration()).unwrap();
    assert_ne!(first.state().module().origin(), second.state().module().origin());
    assert_eq!(first.state().module().image().raw(), second.state().module().image().raw());
    assert_eq!(first.state().import_issuances()[0].word(), second.state().import_issuances()[0].word());
    assert_eq!(first.state().dependencies()[0].module_word(), second.state().dependencies()[0].module_word());
    let foreign_word = first.state.imports[0].word();
    assert!(second.state.services.program_get_module_handle_a(first.state.access.as_ref().unwrap(),
        second.state.plan.module(), foreign_word, Some(b"kernel32")).is_err());
    assert!(first.state.services.program_get_module_handle_a(first.state.access.as_ref().unwrap(),
        second.state.plan.module(), foreign_word, Some(b"kernel32")).is_err());
    // Register the very same actual Module with another real service to
    // isolate service issuance identity from the separate-read origin guard.
    let mut foreign = fixture.services();
    let foreign_access = foreign.begin_program(first.state.plan.module(), configuration()).unwrap();
    foreign.program_load_kernel32(&foreign_access, first.state.plan.module()).unwrap();
    let foreign_import = foreign.program_lease_import(&foreign_access, first.state.plan.module(),
        b"kernel32", ProcedureQuery::Name(b"GetModuleHandleA")).unwrap();
    assert_eq!(foreign_import, foreign_word);
    assert!(first.state.services.program_get_module_handle_a(&foreign_access,
        first.state.plan.module(), foreign_word, Some(b"kernel32")).is_err());
    assert!(foreign.program_get_module_handle_a(first.state.access.as_ref().unwrap(),
        first.state.plan.module(), foreign_import, Some(b"kernel32")).is_err());
    first.advance().unwrap();
    second.advance().unwrap();
    let old_call = first.pending_call().unwrap().clone();
    assert!(!second.accepts_call(&old_call));
    drop(first);
    assert!(!second.accepts_call(&old_call));
    let mut reread = fixture.plan().install(fixture.services(), configuration()).unwrap();
    assert_ne!(reread.state().module().origin(), old_call.origin());
    reread.advance().unwrap();
    assert!(!reread.accepts_call(&old_call));
    assert!(!reread.pending_call().unwrap().same_call(&old_call));
}

#[test]
fn installer_retains_unlinked_or_replaced_path_without_rereading() {
    for replacement in [false, true] {
        let fixture = Fixture::new(&five());
        let plan = fixture.plan();
        let snapshot = Snapshot::new(&plan);
        if replacement {
            // The new pathname contents are deliberately not a supported PE.
            // A pathname reread during installation would fail this control.
            let new_path = fixture.directory.join("replacement.exe");
            fs::write(&new_path, b"authored replacement, not a PE").unwrap();
            fs::rename(new_path, &fixture.path).unwrap();
        } else {
            fs::remove_file(&fixture.path).unwrap();
        }
        let installed = plan.install(fixture.services(), configuration()).unwrap();
        assert_eq!(installed.state().module().observation().supplied_path, fixture.path);
        snapshot.exact_effects(installed.state());
        assert_eq!(installed.state().import_issuances().len(), 5);
    }
}

fn installer_registry_limits() -> program_registry_store::Limits {
    program_registry_store::Limits { max_bytes: 1 << 20, max_keys: 128, max_records: 128 }
}
fn installer_registry_hive(case: &Fixture) -> program_registry_store::Hive {
    program_registry_store::Hive::create_new(&case.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("registry.hive"), installer_registry_limits()).unwrap()
}
fn registry_install<'a>(plan: ImportPlan, services: BootstrapServices<'a>, hive: program_registry_store::Hive)
    -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>>
{
    plan.install_with_private_user_registry(services, configuration(), program_ole32::Policy::Disabled,
        program_winmm::Policy::Disabled, hive, 32)
}

#[test]
fn installer_registry_real_lease_dispatch_and_repeated_descriptors_share_actual_table() {
    let key = [Symbol::Name(b"RegCreateKeyA"), Symbol::Name(b"RegCloseKey")];
    let kernel = EXPORTS.map(Symbol::Name);
    let case = Fixture::new(&fixture(&[(b"KERNEL32.dll", &kernel), (b"ADVAPI32", &[]),
        (b"advapi32.dll", &key), (b"AdVaPi32", &[])], true));
    let plan = case.plan();
    let snapshot = Snapshot::new(&plan);
    let mut installed = registry_install(plan, case.services(), installer_registry_hive(&case)).unwrap();
    let dependencies = installed.state().dependencies();
    assert_eq!(dependencies.len(), 4);
    assert_eq!(dependencies[0].kind(), ModuleKind::Kernel32);
    for index in 1..4 {
        assert_eq!(dependencies[index].kind(), ModuleKind::Advapi32);
        assert_eq!(dependencies[index].module_word(), dependencies[1].module_word());
        assert_eq!(dependencies[index].identity(), dependencies[1].identity());
        assert_eq!(dependencies[index].loaded_here(), index == 1);
    }
    assert_ne!(dependencies[0].identity(), dependencies[1].identity());
    assert_eq!(installed.state().import_issuances().len(), 7);
    snapshot.exact_effects(installed.state());
    let create = installed.state().import_issuances()[5].word();
    let close = installed.state().import_issuances()[6].word();
    let before = fs::read(case.directory.join("registry.hive")).unwrap();
    let state = &mut installed.state;
    let access = state.access.as_ref().unwrap();
    assert_eq!(state.services.program_private_registry_observation(access, state.plan.module()).unwrap().keys, 1);
    let text = std::ffi::CString::new("Software\\AuthoredFixture").unwrap();
    let key = state.services.program_reg_create_key_a(access, state.plan.module(), create,
        crate::native_bootstrap::services::program_registry::PredefinedRoot::CurrentUser.word(),
        crate::native_bootstrap::services::program_registry::SubKey::Ansi(&text)).unwrap();
    state.services.program_reg_close_key(access, state.plan.module(), close, key).unwrap();
    assert_ne!(fs::read(case.directory.join("registry.hive")).unwrap(), before);
    assert_eq!(state.services.program_private_registry_observation(access, state.plan.module()).unwrap().keys, 3);
    snapshot.exact_effects(installed.state());
    drop(installed);
    let reopened = program_registry_store::Hive::open(&case.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("registry.hive"), installer_registry_limits()).unwrap();
    assert_eq!((reopened.snapshot().keys, reopened.snapshot().records), (3, 1));
}

#[test]
fn installer_registry_late_unknown_retains_actual_store_and_has_zero_image_effects() {
    let key = [Symbol::Name(b"RegCreateKeyA")];
    let kernel = EXPORTS.map(Symbol::Name);
    let case = Fixture::new(&fixture(&[(b"KERNEL32", &kernel), (b"ADVAPI32", &key),
        (b"GDI32.dll", &[])], false));
    let plan = case.plan();
    let snapshot = Snapshot::new(&plan);
    let hive = installer_registry_hive(&case);
    let before = fs::read(case.directory.join("registry.hive")).unwrap();
    let failure = registry_install(plan, case.services(), hive).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(2));
    assert_eq!(failure.state().dependencies().len(), 2);
    assert_eq!(failure.state().import_issuances().len(), 6);
    assert_eq!(failure.state().services().live_capabilities(), 8);
    snapshot.refused(&failure);
    assert_eq!(fs::read(case.directory.join("registry.hive")).unwrap(), before);
    assert!(program_registry_store::Hive::open(&case.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("registry.hive"), installer_registry_limits()).is_err());
    drop(failure);
    assert_eq!(program_registry_store::Hive::open(&case.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("registry.hive"), installer_registry_limits()).unwrap().snapshot().keys, 1);
}

#[test]
fn installer_registry_preparation_and_begin_refusals_retain_unregistered_hive() {
    for fail_preparation in [false, true] {
        let case = Fixture::new(&five());
        let mut plan = case.plan();
        if fail_preparation {
            plan.slots[0].initial_word ^= 1; // Corrupt preflight metadata, never the owning image.
        }
        let snapshot = Snapshot::new(&plan);
        let mut config = configuration();
        if !fail_preparation { config.limits.module_issuances = 0; }
        let failure = plan.install_with_private_user_registry(case.services(), config,
            program_ole32::Policy::Disabled, program_winmm::Policy::Disabled,
            installer_registry_hive(&case), 32).unwrap_err();
        assert_eq!(failure.stage, if fail_preparation { InstallStage::Preparation } else { InstallStage::BeginProgram });
        assert!(!failure.state().registered_by_install());
        assert!(failure.state.pending_registry.is_some());
        assert!(failure.state().dependencies().is_empty());
        snapshot.refused(&failure);
        assert!(program_registry_store::Hive::open(&case.directory.canonicalize().unwrap(),
            std::ffi::OsStr::new("registry.hive"), installer_registry_limits()).is_err());
        drop(failure);
        assert_eq!(program_registry_store::Hive::open(&case.directory.canonicalize().unwrap(),
            std::ffi::OsStr::new("registry.hive"), installer_registry_limits()).unwrap().snapshot().keys, 1);
    }
}

#[test]
fn installer_registry_old_profiles_remain_disabled_and_wrong_table_never_writes() {
    let key = [Symbol::Name(b"RegCreateKeyA")];
    let case = Fixture::new(&fixture(&[(b"ADVAPI32", &key)], false));
    let plan = case.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = plan.install_with_profiles(case.services(), configuration(), program_ole32::Policy::OwnedRustSta,
        program_winmm::Policy::Disabled).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(0));
    assert!(failure.state().dependencies().is_empty());
    snapshot.refused(&failure);
    let wrong = Fixture::new(&fixture(&[(b"ADVAPI32", &[Symbol::Name(b"GlobalAlloc")])], false));
    let plan = wrong.plan();
    let snapshot = Snapshot::new(&plan);
    let failure = registry_install(plan, wrong.services(), installer_registry_hive(&wrong)).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Import(0));
    assert_eq!(failure.state().dependencies()[0].kind(), ModuleKind::Advapi32);
    assert!(failure.state().import_issuances().is_empty());
    snapshot.refused(&failure);
}

#[test]
#[ignore = "parent-only exact hash-pinned initial installed executable; no guest execution"]
fn installer_registry_parent_installed_input_reaches_gdi_dependency2_without_image_effects() {
    let path = std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE")
        .expect("parent supplies the fixed hash-verified initial executable copy");
    let module = read(std::path::Path::new(&path)).unwrap();
    assert_eq!(module.observation().bytes_read, 4_422_663);
    let digest: String = module.observation().sha256.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest, "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
    let plan = module.begin_initial_imports().unwrap();
    assert_eq!((plan.len(), plan.descriptor_count()), (13, 9));
    let snapshot = Snapshot::new(&plan);
    let case = Fixture::new(&five());
    let failure = registry_install(plan, case.services(), installer_registry_hive(&case)).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(2));
    assert_eq!(failure.state().dependencies().len(), 2);
    assert_eq!(failure.state().import_issuances().len(), 6);
    assert_eq!(failure.state().services().live_capabilities(), 8);
    assert_eq!(failure.state().dependencies()[1].kind(), ModuleKind::Advapi32);
    snapshot.refused(&failure);
}

fn installer_registry_assert_lock(case: &Fixture, leaf: &str, busy: bool) {
    let file = fs::OpenOptions::new().read(true).write(true).open(case.directory.join(leaf)).unwrap();
    // SAFETY: the exact fixture file has a separately opened live descriptor;
    // flock takes no pointers, and this attempt is explicitly nonblocking.
    let result = unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_EX | libc::LOCK_NB) };
    if busy {
        let error = std::io::Error::last_os_error();
        assert_eq!(result, -1, "uncertain owner released its independent descriptor lock");
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    } else {
        assert_eq!(result, 0, "ordinary owner drop did not release its lock");
    }
}

#[test]
fn installer_registry_pending_producer_begin_refusal_retains_exact_poisoned_hive() {
    let key = [Symbol::Name(b"RegCreateKeyA")];
    let case = Fixture::new(&fixture(&[(b"ADVAPI32", &key)], false));
    let plan = case.plan(); let snapshot = Snapshot::new(&plan);
    let mut hive = installer_registry_hive(&case); let root = hive.root().unwrap();
    hive.create(&root, b"Existing").unwrap();
    hive.inject_parent_test_write_failure_once(false);
    assert!(hive.create(&root, b"Uncertain").is_err());
    let pending = hive.snapshot(); assert!(pending.poisoned && pending.pending_write);
    let bytes = fs::read(case.directory.join("registry.hive")).unwrap();
    let failure = registry_install(plan, case.services(), hive).unwrap_err();
    assert_eq!(failure.stage, InstallStage::BeginProgram);
    assert!(!failure.state().registered_by_install());
    assert_eq!(failure.state.pending_registry.as_ref().unwrap().0.snapshot(), pending);
    assert!(failure.state().dependencies().is_empty()); snapshot.refused(&failure);
    assert_eq!(fs::read(case.directory.join("registry.hive")).unwrap(), bytes);
    installer_registry_assert_lock(&case, "registry.hive", true);
    drop(failure);
    installer_registry_assert_lock(&case, "registry.hive", false);
    assert_eq!(fs::read(case.directory.join("registry.hive")).unwrap(), bytes);
}

#[test]
fn installer_registry_existing_program_refusal_retains_both_distinct_hive_owners() {
    let key = [Symbol::Name(b"RegCreateKeyA")];
    let case = Fixture::new(&fixture(&[(b"ADVAPI32", &key)], false));
    let original = read(&case.path).unwrap(); let mut services = case.services();
    let mut active = program_registry_store::Hive::create_new(&case.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("active.hive"), installer_registry_limits()).unwrap();
    let root = active.root().unwrap(); active.create(&root, b"Existing").unwrap();
    let access = match services.begin_program_with_private_user_registry(&original, configuration(),
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled, active, 16)
    {
        Ok(access) => access,
        Err(error) => panic!("actual original registry refused: {:?}", error.error),
    };
    let plan = case.plan(); let snapshot = Snapshot::new(&plan);
    let failure = registry_install(plan, services, installer_registry_hive(&case)).unwrap_err();
    assert_eq!(failure.stage, InstallStage::BeginProgram);
    assert!(!failure.state().registered_by_install()); snapshot.refused(&failure);
    assert_eq!(failure.state.pending_registry.as_ref().unwrap().0.snapshot().keys, 1);
    assert_eq!(failure.state().services().program_private_registry_observation(&access, &original).unwrap().keys, 2);
    for leaf in ["registry.hive", "active.hive"] { installer_registry_assert_lock(&case, leaf, true); }
    drop(failure);
    for leaf in ["registry.hive", "active.hive"] { installer_registry_assert_lock(&case, leaf, false); }
}

#[test]
fn installer_registry_late_real_procedure_budget_refusal_retains_partial_issuance_without_stores() {
    let keys = [Symbol::Name(b"RegCreateKeyA"), Symbol::Name(b"RegCloseKey")];
    let case = Fixture::new(&fixture(&[(b"ADVAPI32", &keys)], false));
    let plan = case.plan(); let snapshot = Snapshot::new(&plan);
    let mut config = configuration(); config.limits.procedure_lease_issuances = 1;
    let hive = installer_registry_hive(&case);
    let bytes = fs::read(case.directory.join("registry.hive")).unwrap();
    let failure = plan.install_with_private_user_registry(case.services(), config,
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled, hive, 16).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Import(1));
    assert!(failure.state().registered_by_install());
    assert_eq!(failure.state().dependencies().len(), 1);
    assert_eq!(failure.state().dependencies()[0].kind(), ModuleKind::Advapi32);
    assert_eq!(failure.state().import_issuances().len(), 1);
    assert_eq!(failure.state().services().live_capabilities(), 2);
    snapshot.refused(&failure);
    assert_eq!(fs::read(case.directory.join("registry.hive")).unwrap(), bytes);
    installer_registry_assert_lock(&case, "registry.hive", true);
    drop(failure); installer_registry_assert_lock(&case, "registry.hive", false);
}

// Explicit combined private-registry/raster installer controls. AUTHORED UNRUN.
fn installer_gdi_source() -> (program_raster::Device, program_raster::Bitmap) {
    let mut device = program_raster::Device::new(program_raster::Limits {
        max_width: 8, max_height: 8, max_bitmap_bytes: 256,
        max_live_bytes: 4096, max_bitmaps: 64, max_selections: 64,
    }).unwrap();
    let bitmap = device.create_bitmap(program_raster::PixelFormat::Rgba8Straight, 2, 1).unwrap();
    let mut selection = device.new_selection().unwrap();
    device.select(&mut selection, &bitmap).unwrap();
    device.fill_rect(&selection, program_raster::Rect { x: 0, y: 0, width: 2, height: 1 },
        program_raster::Pixel::Rgba([29, 59, 113, 181])).unwrap();
    drop(selection);
    (device, bitmap)
}
fn installer_gdi_combined<'a>(plan: ImportPlan, services: BootstrapServices<'a>, hive: program_registry_store::Hive,
                              device: program_raster::Device)
    -> std::result::Result<InstalledProgram<'a>, InstallFailure<'a>>
{
    plan.install_with_private_user_registry_and_raster(services, configuration(),
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled, hive, 32,
        device, program_raster_context::Policy::PrivateRasterV1, 16)
}

#[test]
fn installer_gdi_combined_real_four_dependency_pe_refuses_user32_without_image_effects() {
    let kernel = EXPORTS.map(Symbol::Name);
    let registry = [Symbol::Name(b"RegCreateKeyA")];
    let raster = [Symbol::Name(b"CreateCompatibleDC")];
    let user = [Symbol::Name(b"GetDesktopWindow")];
    let case = Fixture::new(&fixture(&[(b"KERNEL32.dll", &kernel), (b"ADVAPI32.dll", &registry),
        (b"GDI32.dll", &raster), (b"USER32.dll", &user)], true));
    let plan = case.plan(); let snapshot = Snapshot::new(&plan);
    let hive = installer_registry_hive(&case);
    let hive_bytes = fs::read(case.directory.join("registry.hive")).unwrap();
    let (device, bitmap) = installer_gdi_source();
    let before = device.snapshot().unwrap();
    let mut failure = installer_gdi_combined(plan, case.services(), hive, device).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(3));
    assert_eq!(failure.state().dependencies().len(), 3);
    assert_eq!(failure.state().import_issuances().len(), 7);
    assert_eq!(failure.state().services().live_capabilities(), 10);
    for (index, kind) in [ModuleKind::Kernel32, ModuleKind::Advapi32, ModuleKind::Gdi32].into_iter().enumerate() {
        assert_eq!(failure.state().dependencies()[index].kind(), kind);
        assert!(failure.state().dependencies()[index].loaded_here());
    }
    let state = &failure.state;
    let access = state.access.as_ref().unwrap();
    let observed = state.services.program_private_raster_observation(access, state.module()).unwrap();
    assert_eq!(observed.contexts_issued, 0); assert_eq!(observed.device, before);
    assert_eq!(fs::read(case.directory.join("registry.hive")).unwrap(), hive_bytes);
    installer_registry_assert_lock(&case, "registry.hive", true);
    snapshot.refused(&failure);
    // Test-only access to the retained state exercises its real partial Gdi32
    // import owner; production refusal exposes no mutable-services/retry seam.
    let procedure = failure.state().import_issuances()[6].word();
    let state = &mut failure.state; let access = state.access.as_ref().unwrap();
    let source = state.services.program_raster_attach_source(access, state.plan.module(), &bitmap).unwrap();
    let derived = state.services.program_create_compatible_dc(access, state.plan.module(), procedure, source).unwrap();
    assert_eq!(state.services.program_raster_readback_rgba(access, state.plan.module(), derived).unwrap().pixels,
        [0, 0, 0, 255]);
    state.services.program_raster_close(access, state.plan.module(), source).unwrap();
    state.services.program_raster_close(access, state.plan.module(), derived).unwrap();
    snapshot.refused(&failure);
    drop(failure);
    installer_registry_assert_lock(&case, "registry.hive", false);
    drop(bitmap);
}

#[test]
fn installer_gdi_real_import_dispatch_repeated_tables_and_unsupported_symbols() {
    let raster = [Symbol::Name(b"CreateCompatibleDC")];
    let case = Fixture::new(&fixture(&[(b"GDI32.dll", &[]), (b"gdi32", &raster), (b"GdI32.DlL", &raster)], false));
    let plan = case.plan(); let snapshot = Snapshot::new(&plan);
    let (device, bitmap) = installer_gdi_source();
    let mut installed = plan.install_with_private_raster(case.services(), configuration(),
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled,
        device, program_raster_context::Policy::PrivateRasterV1, 16).unwrap();
    let dependencies = installed.state().dependencies();
    assert_eq!(dependencies.len(), 3);
    for index in 0..3 {
        assert_eq!(dependencies[index].kind(), ModuleKind::Gdi32);
        assert_eq!(dependencies[index].loaded_here(), index == 0);
        assert_eq!(dependencies[index].module_word(), dependencies[0].module_word());
        assert_eq!(dependencies[index].identity(), dependencies[0].identity());
    }
    assert_eq!(installed.state().import_issuances().len(), 2);
    assert_eq!(installed.state().services().live_capabilities(), 3);
    snapshot.exact_effects(installed.state());
    let procedure = installed.state().import_issuances()[0].word();
    let state = &mut installed.state; let access = state.access.as_ref().unwrap();
    assert!(state.services.program_create_compatible_dc(access, state.plan.module(), procedure, 0).is_err());
    let source = state.services.program_raster_attach_source(access, state.plan.module(), &bitmap).unwrap();
    let derived = state.services.program_create_compatible_dc(access, state.plan.module(), procedure, source).unwrap();
    assert_eq!(state.services.program_raster_readback_rgba(access, state.plan.module(), source).unwrap().pixels,
        [29, 59, 113, 181].repeat(2));
    let pixels = state.services.program_raster_readback_rgba(access, state.plan.module(), derived).unwrap();
    assert_eq!((pixels.width, pixels.height, pixels.pixels), (1, 1, vec![0, 0, 0, 255]));
    state.services.program_raster_close(access, state.plan.module(), source).unwrap();
    state.services.program_raster_close(access, state.plan.module(), derived).unwrap();
    snapshot.exact_effects(installed.state());
    for symbol in [Symbol::Name(b"DeleteDC"), Symbol::Name(b"createcompatibledc"), Symbol::Ordinal(1)] {
        let case = Fixture::new(&fixture(&[(b"GDI32", &[symbol])], false));
        let plan = case.plan(); let snapshot = Snapshot::new(&plan);
        let (device, _) = installer_gdi_source();
        let failure = plan.install_with_private_raster(case.services(), configuration(),
            program_ole32::Policy::Disabled, program_winmm::Policy::Disabled,
            device, program_raster_context::Policy::PrivateRasterV1, 16).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Import(0));
        assert_eq!(failure.state().dependencies()[0].kind(), ModuleKind::Gdi32);
        assert!(failure.state().import_issuances().is_empty());
        snapshot.refused(&failure);
    }
}

#[test]
fn installer_gdi_old_apis_preserve_advapi1_and_registry_only_gdi2_refusals() {
    let kernel = EXPORTS.map(Symbol::Name);
    let registry = [Symbol::Name(b"RegCreateKeyA")]; let raster = [Symbol::Name(b"CreateCompatibleDC")];
    let case = Fixture::new(&fixture(&[(b"kernel32", &kernel), (b"advapi32", &registry),
        (b"gdi32", &raster), (b"user32", &[])], false));
    for api in 0..3 {
        let plan = case.plan(); let snapshot = Snapshot::new(&plan);
        let failure = match api {
            0 => plan.install(case.services(), configuration()),
            1 => plan.install_with_ole32(case.services(), configuration(), program_ole32::Policy::OwnedRustSta),
            _ => plan.install_with_profiles(case.services(), configuration(), program_ole32::Policy::OwnedRustSta,
                program_winmm::Policy::DarwinKernelBootElapsed),
        }.unwrap_err();
        assert_eq!(failure.stage, InstallStage::Dependency(1));
        assert_eq!(failure.state().import_issuances().len(), 5);
        snapshot.refused(&failure);
    }
    let plan = case.plan(); let snapshot = Snapshot::new(&plan);
    let failure = registry_install(plan, case.services(), installer_registry_hive(&case)).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(2));
    assert_eq!(failure.state().import_issuances().len(), 6);
    snapshot.refused(&failure);
    for symbols in [raster.as_slice(), &[]] {
        let case = Fixture::new(&fixture(&[(b"GDI32", symbols)], false));
        let plan = case.plan(); let snapshot = Snapshot::new(&plan);
        let failure = plan.install(case.services(), configuration()).unwrap_err();
        assert_eq!(failure.stage, InstallStage::Dependency(0));
        assert!(failure.state().dependencies().is_empty());
        snapshot.refused(&failure);
    }
}

#[test]
fn installer_gdi_preparation_begin_and_enable_refusals_retain_actual_device() {
    for which in 0..6 {
        let case = Fixture::new(&fixture(&[(b"gdi32", &[Symbol::Name(b"CreateCompatibleDC")])], false));
        let mut plan = case.plan();
        if which == 0 { plan.entry.advance().unwrap(); }
        let current = plan.entry.current_image_bytes().to_vec();
        let snapshot = Snapshot::new(&plan);
        let mut config = configuration(); if which == 1 { config.limits.module_issuances = 0; }
        let (device, bitmap) = installer_gdi_source();
        let observed = device.observe(&bitmap).unwrap(); let before = device.snapshot().unwrap();
        let policy = if which == 2 { program_raster_context::Policy::Disabled } else { program_raster_context::Policy::PrivateRasterV1 };
        let max_contexts = if which == 3 { 0 } else if which == 4 { 4097 } else { 16 };
        let original = read(&case.path).unwrap(); let mut services = case.services();
        let existing_access = if which == 5 { Some(services.begin_program(&original, configuration()).unwrap()) } else { None };
        let failure = plan.install_with_private_raster(services, config,
            program_ole32::Policy::Disabled, program_winmm::Policy::Disabled,
            device, policy, max_contexts).unwrap_err();
        assert_eq!(failure.stage, if which == 0 { InstallStage::Preparation } else if which == 1 || which == 5 {
            InstallStage::BeginProgram } else { InstallStage::EnablePrivateRaster });
        let device = &failure.state.pending_raster.as_ref().unwrap().0;
        assert!(device.observe(&bitmap).unwrap().same_bitmap(&observed));
        assert_eq!(device.snapshot().unwrap(), before);
        assert!(failure.state().dependencies().is_empty());
        assert!(failure.state().import_issuances().is_empty());
        if let Some(access) = existing_access {
            assert!(!failure.state().registered_by_install());
            assert!(failure.state.services.program_dependency_kind(&access, &original, b"gdi32").is_err());
        }
        if which != 0 { snapshot.refused(&failure); }
        else {
            assert!(failure.state().iat_writes().is_empty());
            snapshot.immutable(failure.state());
            assert_eq!(failure.state().current_image(), current);
        }
    }
}

#[test]
fn installer_gdi_combined_late_lease_budget_refusal_retains_real_partial_issuances() {
    let kernel = [Symbol::Name(b"GlobalAlloc")]; let key = [Symbol::Name(b"RegCreateKeyA")];
    let raster = [Symbol::Name(b"CreateCompatibleDC"), Symbol::Name(b"CreateCompatibleDC")];
    let case = Fixture::new(&fixture(&[(b"kernel32", &kernel), (b"advapi32", &key), (b"gdi32", &raster)], false));
    let plan = case.plan(); let snapshot = Snapshot::new(&plan);
    let mut config = configuration(); config.limits.procedure_lease_issuances = 3;
    let (device, _) = installer_gdi_source();
    let failure = plan.install_with_private_user_registry_and_raster(case.services(), config,
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled, installer_registry_hive(&case), 32,
        device, program_raster_context::Policy::PrivateRasterV1, 16).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Import(3));
    assert_eq!(failure.state().dependencies().len(), 3);
    assert_eq!(failure.state().import_issuances().len(), 3);
    assert_eq!(failure.state().services().live_capabilities(), 6);
    assert!(failure.state.pending_raster.is_none());
    assert!(failure.state.pending_registry.is_none());
    installer_registry_assert_lock(&case, "registry.hive", true);
    snapshot.refused(&failure);
}

#[test]
#[ignore = "parent-only exact hash-pinned initial installed executable; AUTHORED UNRUN, no guest execution"]
fn installer_gdi_parent_installed_input_reaches_user32_dependency3_without_image_effects() {
    let path = std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE")
        .expect("parent supplies the fixed hash-verified initial executable copy");
    let module = read(std::path::Path::new(&path)).unwrap();
    assert_eq!(module.observation().bytes_read, 4_422_663);
    let digest: String = module.observation().sha256.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(digest, "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
    let plan = module.begin_initial_imports().unwrap();
    assert_eq!((plan.len(), plan.descriptor_count()), (13, 9));
    assert!(plan.descriptors().nth(3).unwrap().library().eq_ignore_ascii_case(b"USER32.dll"));
    let snapshot = Snapshot::new(&plan); let case = Fixture::new(&five());
    let (device, _) = installer_gdi_source();
    let failure = installer_gdi_combined(plan, case.services(), installer_registry_hive(&case), device).unwrap_err();
    assert_eq!(failure.stage, InstallStage::Dependency(3));
    assert_eq!(failure.state().dependencies().len(), 3);
    assert_eq!(failure.state().import_issuances().len(), 7);
    assert_eq!(failure.state().services().live_capabilities(), 10);
    assert_eq!(failure.state().dependencies()[2].kind(), ModuleKind::Gdi32);
    snapshot.refused(&failure);
}
