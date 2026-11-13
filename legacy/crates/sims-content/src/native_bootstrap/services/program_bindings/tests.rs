//! Actual-service/temp-PE integration controls. All authored UNRUN by the
//! isolated implementation worker; parent owns compilation and execution.

use super::*;
use super::super::{CAP_START, MAX_CAPABILITIES, LocalePolicy};
use crate::{Index, Resource, avatar_catalog::Directories};
use program_modules::{
    Configuration, ExitPolicy, Export, FreeFailure, GlobalFreeReturn, History,
    Kernel32Policy, Limits,
};
use sims_sim::native_world::resources::{manager, path};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    executable: PathBuf,
    index: Index,
    directories: Directories,
}

impl Fixture {
    fn new() -> Self {
        let directory = loop {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let candidate = std::env::temp_dir().join(format!("sims-program-bindings-{nanos}-{sequence}"));
            match fs::DirBuilder::new().mode(0o700).create(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create authored fixture directory: {error}"),
            }
        };
        let executable = directory.join("authored.exe");
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o644)
            .open(&executable).unwrap();
        file.write_all(&pe()).unwrap();
        drop(file);
        let mut directories = Directories::default();
        directories.insert_first(b"./GameData/", b"Scenario.tds", 0).unwrap();
        let index = Index {
            root: directory.clone(),
            resources: vec![Resource {
                locator: "./GameData/Scenario.tds".into(),
                name: "Scenario.tds".into(),
                source: directory.join("Scenario.tds"),
                offset: 0,
                size: 11,
                unpacked_size: 11,
                archived: false,
                owned: Some(b"world bytes".to_vec().into()),
            }],
            archives: 0,
            diagnostics: vec![],
        };
        Self { directory, executable, index, directories }
    }

    fn input(&self) -> Module {
        crate::native_bootstrap::executable::read(&self.executable).unwrap()
    }

    fn services(&self, bytes: usize) -> BootstrapServices<'_> {
        BootstrapServices::new(
            super::super::Memory::new(bytes).unwrap(), &self.index, &self.directories, LocalePolicy::C,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

// Authored I386 PE32, one section and sixteen absent directories. These bytes
// go through the real descriptor reader/parser; no Module or origin adoption.
fn pe() -> Vec<u8> {
    let mut bytes = vec![0; 0x400];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 0x84, 0x014c);
    put16(&mut bytes, 0x86, 1);
    put16(&mut bytes, 0x94, 224);
    put16(&mut bytes, 0x96, 0x0102);
    let optional = 0x98;
    put16(&mut bytes, optional, 0x010b);
    put32(&mut bytes, optional + 4, 0x200);
    put32(&mut bytes, optional + 12, 0x100);
    put32(&mut bytes, optional + 16, 0x1000);
    put32(&mut bytes, optional + 20, 0x1000);
    put32(&mut bytes, optional + 24, 0x1000);
    put32(&mut bytes, optional + 28, 0x0040_0000);
    put32(&mut bytes, optional + 32, 0x1000);
    put32(&mut bytes, optional + 36, 0x200);
    put16(&mut bytes, optional + 40, 4);
    put16(&mut bytes, optional + 48, 4);
    put32(&mut bytes, optional + 56, 0x2000);
    put32(&mut bytes, optional + 60, 0x200);
    put16(&mut bytes, optional + 68, 3);
    put32(&mut bytes, optional + 72, 0x10_0000);
    put32(&mut bytes, optional + 76, 0x1000);
    put32(&mut bytes, optional + 80, 0x10_0000);
    put32(&mut bytes, optional + 84, 0x1000);
    put32(&mut bytes, optional + 92, 16);
    let section = optional + 224;
    bytes[section..section + 8].copy_from_slice(b".text\0\0\0");
    put32(&mut bytes, section + 8, 0x300);
    put32(&mut bytes, section + 12, 0x1000);
    put32(&mut bytes, section + 16, 0x200);
    put32(&mut bytes, section + 20, 0x200);
    put32(&mut bytes, section + 36, 0x6000_0020);
    bytes[0x200..0x204].copy_from_slice(&[0x51, 0x72, 0x93, 0xb4]);
    bytes
}

fn configuration() -> Configuration {
    Configuration {
        kernel32: Kernel32Policy::RustPort,
        exit: ExitPolicy::SingleThreadNoGuestDllDetachCallbacks,
        limits: Limits {
            module_issuances: 4,
            procedure_lease_issuances: 64,
            weak_observation_issuances: 64,
            global_allocation_issuances: 16,
            max_live_global_bytes: 4096,
            max_global_bytes: 1024,
            max_name_bytes: 64,
        },
    }
}

fn begin(host: &mut BootstrapServices<'_>, input: &Module) -> (ProgramAccess, u32) {
    let access = host.begin_program(input, configuration()).unwrap();
    let module = host.program_load_kernel32(&access, input).unwrap();
    (access, module)
}

fn lease(host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module, export: Export) -> u32 {
    host.program_lease_import(access, input, b"kernel32.dll", ProcedureQuery::Name(export.name())).unwrap()
}

fn imports(host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module) -> [u32; 5] {
    [Export::GetModuleHandleA, Export::GetProcAddress, Export::GlobalAlloc, Export::GlobalFree, Export::ExitProcess]
        .map(|export| lease(host, access, input, export))
}

fn history(host: &BootstrapServices<'_>) -> History {
    host.program.as_ref().unwrap().registry.history(&host.memory).unwrap()
}

fn state(host: &BootstrapServices<'_>) -> (u32, usize, usize, usize, usize) {
    (
        host.next, host.live_capabilities(), host.memory.live_bytes(), host.memory.live_allocations(),
        host.program.as_ref().map_or(0, |program| program.globals.len()),
    )
}

// WINMM controls, ALL UNRUN. Synthetic controls never make a valid enabled
// dispatch on Darwin64. The one real dispatch control is explicitly ignored.
fn winmm_begin(host: &mut BootstrapServices<'_>, input: &Module) -> (ProgramAccess, u32, u32) {
    let access = host.begin_program_with_profiles(input, configuration(), program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap();
    let module = host.program_load_winmm(&access, input).unwrap();
    let imported = host.program_lease_import(&access, input, b"winmm.dll", ProcedureQuery::Name(b"timeGetTime")).unwrap();
    (access, module, imported)
}
fn winmm_unsampled(host: &BootstrapServices<'_>, access: &ProgramAccess, input: &Module) {
    let observation = host.program_time_get_time_observation(access, input).unwrap();
    assert!(observation.last_sample().is_none());
    assert!(observation.latest_report().is_none());
    assert!(observation.first_failure_report().is_none());
}

#[test]
fn winmm_old_begin_apis_disable_and_cannot_reconfigure() {
    let fixture = Fixture::new();
    let input = fixture.input();
    for ole32_api in [false, true] {
        let mut host = fixture.services(8192);
        let access = if ole32_api {
            host.begin_program_with_ole32(&input, configuration(), program_ole32::Policy::OwnedRustSta).unwrap()
        } else { host.begin_program(&input, configuration()).unwrap() };
        let before = (state(&host), history(&host));
        assert!(host.program_load_winmm(&access, &input).is_err());
        assert!(host.program_dependency_kind(&access, &input, b"winmm").is_err());
        assert!(host.program_lease_import(&access, &input, b"winmm", ProcedureQuery::Name(b"timeGetTime")).is_err());
        assert!(host.begin_program_with_profiles(&input, configuration(), program_ole32::Policy::OwnedRustSta,
            program_winmm::Policy::DarwinKernelBootElapsed).is_err());
        assert_eq!((state(&host), history(&host)), before);
        assert_eq!(host.program_time_get_time_observation(&access, &input).unwrap().policy(), program_winmm::Policy::Disabled);
        winmm_unsampled(&host, &access, &input);
    }
}

#[test]
fn winmm_foreign_access_input_heap_and_wrong_capabilities_refuse_before_sampling() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let same_bytes = fixture.input();
    let mut host = fixture.services(8192);
    let mut foreign = fixture.services(8192);
    let (access, module, imported) = winmm_begin(&mut host, &input);
    let (other_access, _, other_import) = winmm_begin(&mut foreign, &input);
    assert_eq!(imported, other_import);
    assert_ne!(host.program_module_identity(&access, &input, module).unwrap(),
        foreign.program_module_identity(&other_access, &input, module).unwrap());
    host.program_load_kernel32(&access, &input).unwrap();
    let wrong_export = lease(&mut host, &access, &input, Export::GlobalFree);
    let file = match path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap() {
        path::Opened::File(word) => u32::try_from(word).unwrap(),
        _ => panic!("authored mounted file must resolve"),
    };
    let before = (state(&host), history(&host));
    assert!(host.program_time_get_time(&other_access, &input, imported).is_err());
    assert!(foreign.program_time_get_time(&access, &input, other_import).is_err());
    assert!(host.program_time_get_time(&access, &same_bytes, imported).is_err());
    for word in [0, u32::MAX, module, wrong_export, file] {
        assert!(host.program_time_get_time(&access, &input, word).is_err());
    }
    assert!(host.program_time_get_time_observation(&other_access, &input).is_err());
    assert!(host.program_time_get_time_observation(&access, &same_bytes).is_err());
    assert_eq!((state(&host), history(&host)), before);
    winmm_unsampled(&host, &access, &input);
    winmm_unsampled(&foreign, &other_access, &input);
    let original = std::mem::replace(&mut host.memory, Memory::new(8192).unwrap());
    let swapped = state(&host);
    assert!(host.program_time_get_time(&access, &input, imported).is_err());
    assert!(host.program_time_get_time_observation(&access, &input).is_err());
    assert_eq!(state(&host), swapped);
    host.memory = original;
    assert_eq!((state(&host), history(&host)), before);
    winmm_unsampled(&host, &access, &input);
}

#[test]
fn winmm_weak_words_retire_while_actual_imports_pin_and_terminal_keeps_diagnostics() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let access = host.begin_program_with_profiles(&input, configuration(), program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap();
    let module = host.program_load_winmm(&access, &input).unwrap();
    host.program_load_kernel32(&access, &input).unwrap();
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let weak = host.program_get_proc_address(&access, &input, get_proc, module,
        ProcedureQuery::Name(b"timeGetTime")).unwrap();
    let before = history(&host);
    assert!(matches!(host.capabilities.get(&weak), Some(Capability::ProgramProcedure(_))));
    host.program.as_mut().unwrap().registry.retire_winmm(&host.memory).unwrap();
    assert!(host.program_time_get_time(&access, &input, weak).is_err());
    let fresh = host.program_load_winmm(&access, &input).unwrap();
    assert_ne!(module, fresh);
    assert!(host.program_time_get_time(&access, &input, weak).is_err());
    assert_eq!(history(&host).procedure_leases_issued, before.procedure_leases_issued);
    let imported = host.program_lease_import(&access, &input, b"WINMM", ProcedureQuery::Name(b"timeGetTime")).unwrap();
    assert!(matches!(host.capabilities.get(&imported), Some(Capability::ProgramImport(_))));
    assert!(host.program.as_mut().unwrap().registry.retire_winmm(&host.memory).is_err());
    winmm_unsampled(&host, &access, &input);
    let exit = lease(&mut host, &access, &input, Export::ExitProcess);
    assert_eq!(host.program_exit_process(&access, &input, exit, 12).unwrap().exit_code, 12);
    assert!(host.program_time_get_time(&access, &input, imported).is_err());
    assert!(host.program_time_get_time(&access, &input, weak).is_err());
    assert!(host.program_load_winmm(&access, &input).is_err());
    assert!(host.begin_program_with_profiles(&input, configuration(), program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).is_err());
    winmm_unsampled(&host, &access, &input);
    assert_eq!(host.live_capabilities(), 0);
}

#[test]
fn winmm_issuance_budget_refusals_preserve_actual_service_and_clock() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut config = configuration();
    config.limits.procedure_lease_issuances = 1;
    let access = host.begin_program_with_profiles(&input, config, program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap();
    host.program_load_winmm(&access, &input).unwrap();
    host.program_lease_import(&access, &input, b"winmm", ProcedureQuery::Name(b"timeGetTime")).unwrap();
    let before = (state(&host), history(&host));
    assert!(host.program_lease_import(&access, &input, b"winmm", ProcedureQuery::Name(b"timeGetTime")).is_err());
    assert_eq!((state(&host), history(&host)), before);
    // Use actual issuances to exhaust the existing shared capability budget.
    // No counter mutation, forged sample, production injection seam or I/O.
    let mut full = fixture.services(8192);
    let mut config = configuration();
    config.limits.procedure_lease_issuances = 4096;
    let access_full = full.begin_program_with_profiles(&input, config, program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap();
    full.program_load_winmm(&access_full, &input).unwrap();
    for _ in 1..MAX_CAPABILITIES {
        full.program_lease_import(&access_full, &input, b"winmm", ProcedureQuery::Name(b"timeGetTime")).unwrap();
    }
    let before_full = (state(&full), history(&full));
    assert!(full.program_lease_import(&access_full, &input, b"winmm", ProcedureQuery::Name(b"timeGetTime")).is_err());
    assert_eq!((state(&full), history(&full)), before_full);
    winmm_unsampled(&host, &access, &input);
    winmm_unsampled(&full, &access_full, &input);
}

#[cfg(not(all(target_os = "macos", target_pointer_width = "64")))]
#[test]
fn winmm_service_actual_host_refusal_has_bounded_typed_reports_and_no_word_success() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, imported) = winmm_begin(&mut host, &input);
    let before = (state(&host), history(&host));
    for _ in 0..32 {
        assert!(matches!(host.program_time_get_time(&access, &input, imported), Err(Stop::Unsupported(_))));
        let evidence = host.program_time_get_time_observation(&access, &input).unwrap();
        assert_eq!(evidence.latest_report().unwrap().result(), Err(program_winmm::Stop::UnsupportedHost));
        assert_eq!(evidence.latest_report(), evidence.first_failure_report());
        assert_eq!(evidence.latest_report().unwrap().sequence(), None);
        assert!(evidence.last_sample().is_none());
    }
    assert_eq!((state(&host), history(&host)), before);
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
#[test]
#[ignore = "parent-only explicit opt-in real WINMM clock dispatch; authored UNRUN"]
fn winmm_parent_real_dispatch_weak_then_import_preserves_sample_reports_and_clock_on_reload() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let access = host.begin_program_with_profiles(&input, configuration(), program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap();
    let old_module = host.program_load_winmm(&access, &input).unwrap();
    host.program_load_kernel32(&access, &input).unwrap();
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let weak = host.program_get_proc_address(&access, &input, get_proc, old_module,
        ProcedureQuery::Name(b"timeGetTime")).unwrap();
    winmm_unsampled(&host, &access, &input);
    let before = (state(&host), history(&host));
    let first = host.program_time_get_time(&access, &input, weak);
    let evidence = host.program_time_get_time_observation(&access, &input).unwrap();
    let value = first.unwrap_or_else(|stop| panic!("parent clock refusal {stop:?}; latest={:?}; first={:?}",
        evidence.latest_report(), evidence.first_failure_report()));
    let sample = evidence.last_sample().unwrap().clone();
    let first_failure = evidence.first_failure_report().copied();
    assert_eq!(value, sample.value()); // zero is accepted without special casing
    assert_eq!(evidence.latest_report(), Some(sample.report()));
    assert_eq!(sample.report().result().unwrap().value(), value);
    assert_eq!(sample.sequence(), 1);
    assert_eq!(sample.report().provenance(), program_winmm::Provenance::DarwinSdkRealApis);
    assert_eq!((state(&host), history(&host)), before);
    host.program.as_mut().unwrap().registry.retire_winmm(&host.memory).unwrap();
    let new_module = host.program_load_winmm(&access, &input).unwrap();
    assert_ne!(old_module, new_module);
    let evidence = host.program_time_get_time_observation(&access, &input).unwrap();
    assert_eq!(evidence.last_sample().unwrap().sequence(), sample.sequence());
    assert_eq!(evidence.latest_report(), Some(sample.report()));
    assert_eq!(evidence.first_failure_report().copied(), first_failure);
    assert!(host.program_time_get_time(&access, &input, weak).is_err());
    assert_eq!(host.program_time_get_time_observation(&access, &input).unwrap().latest_report(), Some(sample.report()));
    let imported = host.program_lease_import(&access, &input, b"winmm", ProcedureQuery::Name(b"timeGetTime")).unwrap();
    let second = host.program_time_get_time(&access, &input, imported);
    let evidence = host.program_time_get_time_observation(&access, &input).unwrap();
    match second {
        Ok(word) => {
            let next = evidence.last_sample().unwrap();
            assert_eq!(word, next.value());
            assert_eq!(next.sequence(), sample.sequence() + 1);
            assert!(next.elapsed_microseconds() >= sample.elapsed_microseconds());
            assert_eq!(evidence.latest_report(), Some(next.report()));
        }
        Err(_) => {
            assert!(evidence.latest_report().unwrap().result().is_err());
            assert_eq!(evidence.latest_report().unwrap().sequence(), Some(sample.sequence() + 1));
            assert_eq!(evidence.last_sample().unwrap().sequence(), sample.sequence());
            assert_eq!(evidence.first_failure_report(), evidence.latest_report());
        }
    }
    let latest = evidence.latest_report().copied();
    let failure = evidence.first_failure_report().copied();
    let exit = lease(&mut host, &access, &input, Export::ExitProcess);
    host.program_exit_process(&access, &input, exit, 0).unwrap();
    assert!(host.program_time_get_time(&access, &input, imported).is_err());
    let final_evidence = host.program_time_get_time_observation(&access, &input).unwrap();
    assert_eq!(final_evidence.latest_report().copied(), latest);
    assert_eq!(final_evidence.first_failure_report().copied(), failure);
}

// OLE32 controls are UNRUN. Build the private COM Request through public
// Construction and let that cursor call the real issuing BootstrapServices.
fn ole32_incoming(host: &mut BootstrapServices<'_>) -> sims_sim::native_application::Incoming {
    use sims_sim::native_world_children::Owner;
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
    for owner in [&app, &argument, &caller, &nt_name, &local_name] {
        memory.synchronize_owner(owner).unwrap();
    }
    sims_sim::native_application::Incoming {
        application: app, argument: argument.token,
        tree_stack: [caller.token + 2, caller.token + 6], mutex_temporary: caller.token + 16,
        mutex_name_globals: Owner { token: 0x6c9f70,
            data: [nt_name.token as u32, local_name.token as u32].into_iter().flat_map(u32::to_le_bytes).collect() },
        mutex_singleton: Owner { token: 0x6f2a5c, data: 0x11223344_u32.to_le_bytes().to_vec() },
        retained: vec![argument, caller, nt_name, local_name], registry: Default::default(),
        registry_globals: Owner { token: 0x6de858, data: vec![0; 8] },
        application_singleton: Owner { token: 0x6f2a58, data: 0xaabbccdd_u32.to_le_bytes().to_vec() },
    }
}

fn ole32_initialize(host: &mut BootstrapServices<'_>) -> (
    sims_sim::native_application::Construction,
    sims_sim::native_application::com::Request,
    sims_sim::native_application::com::Return,
) {
    use sims_sim::native_application::{Call, Construction};
    let mut construction = Construction::new(ole32_incoming(host)).unwrap();
    let mut pending = None;
    for _ in 0..4096 {
        if let Some(Call::CoInitialize(request)) = construction.pending_call() {
            pending = Some(request);
            break;
        }
        construction.step(&mut host.application_host()).unwrap();
    }
    let request = pending.expect("public Construction issues its genuine COM request");
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
    panic!("Construction must retain its genuine COM return");
}

fn ole32_begin(host: &mut BootstrapServices<'_>, input: &Module) -> (ProgramAccess, u32, u32) {
    let access = host.begin_program_with_ole32(input, configuration(), program_ole32::Policy::OwnedRustSta).unwrap();
    let module = host.program_load_ole32(&access, input).unwrap();
    let procedure = host.program_lease_import(&access, input, b"OLE32.dll", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    (access, module, procedure)
}

#[test]
fn ole32_old_begin_cannot_enable_or_reconfigure_com_and_preserves_genuine_apartment() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8 * 1024 * 1024);
    let (construction, request, returned) = ole32_initialize(&mut host);
    let access = host.begin_program(&input, configuration()).unwrap();
    let before = (state(&host), history(&host));
    assert!(host.program_load_ole32(&access, &input).is_err());
    assert!(host.program_dependency_kind(&access, &input, b"ole32.dll").is_err());
    assert!(host.program_lease_import(&access, &input, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).is_err());
    assert!(host.begin_program_with_ole32(&input, configuration(), program_ole32::Policy::OwnedRustSta).is_err());
    assert!(host.program_co_uninitialize(&access, &input, CAP_START, &returned).is_err());
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.apartment(&returned).unwrap().initialization_count(), 1);
    construction.validate().unwrap();
    returned.validate(&request).unwrap();
}

#[test]
fn ole32_genuine_import_void_balances_nested_s_false_once_and_preserves_history() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8 * 1024 * 1024);
    let world = host.memory_mut().allocate_owner(48).unwrap();
    let (first_history, first_request, first) = ole32_initialize(&mut host);
    let (nested_history, nested_request, nested) = ole32_initialize(&mut host);
    assert_eq!((first.hresult(), nested.hresult()), (0, 1));
    let nested_clone = nested.clone();
    let (access, module, procedure) = ole32_begin(&mut host, &input);
    assert_eq!(host.program_module_identity(&access, &input, module).unwrap().kind(), ModuleKind::Ole32);
    let live_first = host.apartment(&first).unwrap();
    let live_nested = host.apartment(&nested_clone).unwrap();
    assert_eq!(live_first.initialization_count(), 2);
    let before = (state(&host), history(&host));
    let (): () = host.program_co_uninitialize(&access, &input, procedure, &nested_clone).unwrap();
    assert!(live_nested.check().is_err());
    assert!(host.apartment(&nested).is_err());
    assert!(nested.current_apartment().is_err());
    live_first.check().unwrap();
    assert_eq!(live_first.initialization_count(), 1);
    for _ in 0..2 {
        assert!(host.program_co_uninitialize(&access, &input, procedure, &nested).is_err());
        assert_eq!(live_first.initialization_count(), 1);
        assert_eq!((state(&host), history(&host)), before);
    }
    nested.validate(&nested_request).unwrap();
    nested_clone.validate(&nested_request).unwrap();
    nested_history.validate().unwrap();
    first_history.validate().unwrap();
    first.validate(&first_request).unwrap();
    let (): () = host.program_co_uninitialize(&access, &input, procedure, &first).unwrap();
    assert!(live_first.check().is_err());
    assert!(host.program_co_uninitialize(&access, &input, procedure, &first).is_err());
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.memory().allocation_bytes(world.token).unwrap(), world.data);
    first_history.validate().unwrap();
    nested_history.validate().unwrap();
}

#[test]
fn ole32_foreign_service_access_input_return_and_procedure_refuse_before_balance() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let reread = fixture.input();
    let mut host = fixture.services(8 * 1024 * 1024);
    let mut other = fixture.services(8 * 1024 * 1024);
    let (history_owner, request, returned) = ole32_initialize(&mut host);
    let (foreign_history, _, foreign_return) = ole32_initialize(&mut other);
    let (access, module, procedure) = ole32_begin(&mut host, &input);
    let (foreign_access, _, foreign_procedure) = ole32_begin(&mut other, &input);
    assert_eq!(returned.current_apartment().unwrap().initialization_count(), 2);
    let failed = sims_sim::native_application::com::Return::failed(&request, 0x80010106).unwrap();
    let before = (state(&host), history(&host), state(&other), history(&other));
    assert!(host.program_co_uninitialize(&foreign_access, &input, procedure, &returned).is_err());
    assert!(other.program_co_uninitialize(&access, &input, foreign_procedure, &foreign_return).is_err());
    assert!(host.program_co_uninitialize(&access, &reread, procedure, &returned).is_err());
    assert!(host.program_co_uninitialize(&access, &input, module, &returned).is_err());
    assert!(host.program_co_uninitialize(&access, &input, procedure, &foreign_return).is_err());
    assert!(host.program_co_uninitialize(&access, &input, procedure, &failed).is_err());
    assert_eq!((state(&host), history(&host), state(&other), history(&other)), before);
    // Negative substitution of a genuine issued procedure with the identical
    // export name from another service. Numeric words confer no authority.
    let local = host.capabilities.remove(&procedure).unwrap();
    let foreign = other.capabilities.remove(&foreign_procedure).unwrap();
    host.capabilities.insert(procedure, foreign);
    assert!(host.program_co_uninitialize(&access, &input, procedure, &returned).is_err());
    let foreign = host.capabilities.remove(&procedure).unwrap();
    host.capabilities.insert(procedure, local);
    other.capabilities.insert(foreign_procedure, foreign);
    assert_eq!(host.apartment(&returned).unwrap().initialization_count(), 2);
    assert_eq!(other.apartment(&foreign_return).unwrap().initialization_count(), 2);
    assert_eq!((state(&host), history(&host), state(&other), history(&other)), before);
    history_owner.validate().unwrap();
    foreign_history.validate().unwrap();
    host.program_co_uninitialize(&access, &input, procedure, &returned).unwrap();
    assert_eq!(other.apartment(&foreign_return).unwrap().initialization_count(), 1);
    other.program_co_uninitialize(&foreign_access, &input, foreign_procedure, &foreign_return).unwrap();
}

#[test]
fn ole32_same_request_foreign_service_return_is_not_the_retained_issuance() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8 * 1024 * 1024);
    let mut other = fixture.services(8 * 1024 * 1024);
    let (construction, request, returned) = ole32_initialize(&mut host);
    let other_app = other.memory_mut().allocate_owner(324).unwrap();
    assert_eq!(other_app.token, returned.application());
    // Even the exact genuine Request, same thread and equal logical Owner324
    // address produce a distinct Return in another actual issuing service.
    let foreign = sims_sim::native_application::Host::initialize_com(
        &mut other.application_host(), &request).unwrap();
    foreign.validate(&request).unwrap();
    assert_ne!(foreign, returned);
    assert_eq!((returned.hresult(), foreign.hresult()), (0, 1));
    let (access, _, procedure) = ole32_begin(&mut host, &input);
    let before = (state(&host), history(&host));
    assert!(host.program_co_uninitialize(&access, &input, procedure, &foreign).is_err());
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.apartment(&returned).unwrap().initialization_count(), 2);
    host.program_co_uninitialize(&access, &input, procedure, &returned).unwrap();
    assert_eq!(other.apartment(&foreign).unwrap().initialization_count(), 1);
    construction.validate().unwrap();
    returned.validate(&request).unwrap();
}

#[test]
fn ole32_heap_and_exact_live_application_owner_are_checked_before_balance() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8 * 1024 * 1024);
    let (construction, _, returned) = ole32_initialize(&mut host);
    let (access, _, procedure) = ole32_begin(&mut host, &input);
    let app = returned.application();
    // A different real heap contains an equal-address complete Owner324.
    let mut foreign_heap = Memory::new(8 * 1024 * 1024).unwrap();
    let foreign_app = foreign_heap.allocate_owner(324).unwrap();
    assert_eq!(app, foreign_app.token);
    let original_heap = std::mem::replace(&mut host.memory, foreign_heap);
    let before = state(&host);
    assert!(host.program_co_uninitialize(&access, &input, procedure, &returned).is_err());
    assert_eq!(state(&host), before);
    assert_eq!(returned.current_apartment().unwrap().initialization_count(), 1);
    host.memory = original_heap;
    let unrelated = host.memory_mut().allocate_owner(324).unwrap();
    // Retargeting only the numeric containing-owner field must not authorize
    // release, even when another live Owner has the same exact extent.
    let capability = host.capabilities.values_mut().find(|cap| matches!(cap,
        Capability::Apartment { returned: held, .. } if held == &returned)).unwrap();
    let Capability::Apartment { application, .. } = capability else { unreachable!() };
    *application = unrelated.token;
    let before = (state(&host), history(&host));
    assert!(host.program_co_uninitialize(&access, &input, procedure, &returned).is_err());
    assert_eq!((state(&host), history(&host)), before);
    let capability = host.capabilities.values_mut().find(|cap| matches!(cap,
        Capability::Apartment { returned: held, .. } if held == &returned)).unwrap();
    let Capability::Apartment { application, .. } = capability else { unreachable!() };
    *application = app;
    host.memory_mut().release_owner(app).unwrap();
    let before = (state(&host), history(&host));
    assert!(host.program_co_uninitialize(&access, &input, procedure, &returned).is_err());
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(returned.current_apartment().unwrap().initialization_count(), 1);
    construction.validate().unwrap();
}

#[test]
fn ole32_weak_dispatch_uses_actual_table_without_pinning_and_retirement_refuses_effects() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8 * 1024 * 1024);
    let (_, _, first) = ole32_initialize(&mut host);
    let (_, _, nested) = ole32_initialize(&mut host);
    let access = host.begin_program_with_ole32(&input, configuration(), program_ole32::Policy::OwnedRustSta).unwrap();
    let module = host.program_load_ole32(&access, &input).unwrap();
    host.program_load_kernel32(&access, &input).unwrap();
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let weak = host.program_get_proc_address(&access, &input, get_proc, module,
        ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    assert_eq!(history(&host).procedure_leases_issued, 1);
    let before = (state(&host), history(&host));
    host.program_co_uninitialize(&access, &input, weak, &nested).unwrap();
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.apartment(&first).unwrap().initialization_count(), 1);
    host.program.as_mut().unwrap().registry.retire_ole32(&host.memory).unwrap();
    assert!(host.program_co_uninitialize(&access, &input, weak, &first).is_err());
    assert_eq!(host.apartment(&first).unwrap().initialization_count(), 1);
    let fresh_module = host.program_load_ole32(&access, &input).unwrap();
    assert_ne!(fresh_module, module);
    assert!(host.program_co_uninitialize(&access, &input, weak, &first).is_err());
    let imported = host.program_lease_import(&access, &input, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    host.program_co_uninitialize(&access, &input, imported, &first).unwrap();
}

#[test]
fn ole32_released_owner_drops_do_not_touch_later_generation_and_fallback_drop_balances() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut old = fixture.services(8 * 1024 * 1024);
    let (old_history, request, returned) = ole32_initialize(&mut old);
    let retained = returned.clone();
    let (access, _, procedure) = ole32_begin(&mut old, &input);
    old.program_co_uninitialize(&access, &input, procedure, &returned).unwrap();
    let mut later = fixture.services(8 * 1024 * 1024);
    let (later_history, _, live) = ole32_initialize(&mut later);
    assert_eq!(live.hresult(), 0);
    assert!(old.program_co_uninitialize(&access, &input, procedure, &retained).is_err());
    returned.validate(&request).unwrap();
    old_history.validate().unwrap();
    drop((old_history, returned, retained, old));
    assert_eq!(later.apartment(&live).unwrap().initialization_count(), 1);
    // Fallback release occurs only at the last genuine Return owner, not at
    // ProgramAccess Drop or when a single Construction/service owner drops.
    let mut next = fixture.services(8 * 1024 * 1024);
    let (next_history, _, next_return) = ole32_initialize(&mut next);
    assert_eq!(next_return.hresult(), 1);
    drop(later);
    drop(later_history);
    assert_eq!(next.apartment(&next_return).unwrap().initialization_count(), 2);
    drop(live);
    assert_eq!(next.apartment(&next_return).unwrap().initialization_count(), 1);
    drop((next_history, next_return, next));
    let mut fresh = fixture.services(8 * 1024 * 1024);
    let (_, _, fresh_return) = ole32_initialize(&mut fresh);
    assert_eq!(fresh_return.hresult(), 0);
}

#[test]
fn ole32_exit_process_does_not_balance_com_or_destroy_world_and_application() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8 * 1024 * 1024);
    let (construction, _, returned) = ole32_initialize(&mut host);
    let world = host.memory_mut().allocate_owner(48).unwrap();
    let (access, _, procedure) = ole32_begin(&mut host, &input);
    host.program_load_kernel32(&access, &input).unwrap();
    let exit = lease(&mut host, &access, &input, Export::ExitProcess);
    let terminal = host.program_exit_process(&access, &input, exit, 12).unwrap();
    assert_eq!((terminal.exit_code, terminal.released_globals), (12, 0));
    assert_eq!(host.apartment(&returned).unwrap().initialization_count(), 1);
    assert!(host.program_co_uninitialize(&access, &input, procedure, &returned).is_err());
    assert_eq!(host.memory().allocation_bytes(world.token).unwrap(), world.data);
    assert_eq!(host.memory().allocation_bytes(returned.application()).unwrap().len(), 324);
    construction.validate().unwrap();
}

// Exercise every public program operation after an authority/lifecycle failure.
// Lease issuance would change state if a same-word context were accidentally
// accepted. The real backend receives every valid dispatch in the other tests.
fn all_refused(
    host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module,
    module: u32, calls: [u32; 5], global: u32,
) {
    let before = state(host);
    assert!(host.program_load_kernel32(access, input).is_err());
    assert!(host.program_lease_import(access, input, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert!(host.program_get_module_handle_a(access, input, calls[0], Some(b"kernel32")).is_err());
    assert!(host.program_get_proc_address(access, input, calls[1], module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert!(host.program_global_alloc(access, input, calls[2], 0, 8).is_err());
    assert!(host.program_global_free(access, input, calls[3], global).is_err());
    assert!(host.program_exit_process(access, input, calls[4], 7).is_err());
    assert_eq!(state(host), before);
}

#[test]
fn explicit_begin_retains_constructor_behavior_and_installs_nothing() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    assert!(host.program.is_none());
    assert_eq!(state(&host), (CAP_START, 0, 0, 0, 0));
    let access = host.begin_program(&input, configuration()).unwrap();
    assert_eq!(history(&host), History {
        modules_issued: 0, procedure_leases_issued: 0, weak_observations_issued: 0,
        global_allocations_issued: 0, live_global_bytes: 0,
    });
    assert!(host.program_lease_import(&access, &input, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert_eq!(state(&host), (CAP_START, 0, 0, 0, 0));
    assert_eq!(host.program_load_kernel32(&access, &input).unwrap(), CAP_START);
    assert_eq!(history(&host).modules_issued, 1);
    let before = (state(&host), history(&host));
    assert!(host.program_load_kernel32(&access, &input).is_err());
    assert_eq!((state(&host), history(&host)), before);
}

#[test]
fn dropping_access_never_resets_or_reissues_the_program_lifetime() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let access = host.begin_program(&input, configuration()).unwrap();
    drop(access);
    let before = (state(&host), history(&host));
    assert!(host.begin_program(&input, configuration()).is_err());
    assert_eq!((state(&host), history(&host)), before);
    assert!(host.program.is_some());
}

#[test]
fn configuration_failure_and_reconfiguration_preserve_world_and_live_program() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let world = host.memory.allocate_owner(32).unwrap();
    let mut invalid = configuration();
    invalid.limits.global_allocation_issuances = 4097;
    let before = state(&host);
    assert!(host.begin_program(&input, invalid).is_err());
    assert!(host.program.is_none());
    assert_eq!(state(&host), before);
    let (access, _) = begin(&mut host, &input);
    let alloc = lease(&mut host, &access, &input, Export::GlobalAlloc);
    let pointer = host.program_global_alloc(&access, &input, alloc, 0, 17).unwrap();
    let before = (state(&host), history(&host));
    assert!(host.begin_program(&input, invalid).is_err());
    assert!(host.begin_program(&input, configuration()).is_err());
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.memory.allocation_bytes(u64::from(pointer)).unwrap(), &[0; 17]);
    assert_eq!(host.memory.allocation_bytes(world.token).unwrap(), world.data);
}

#[test]
fn disabled_kernel32_policy_is_not_a_default_or_native_success() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut config = configuration();
    config.kernel32 = Kernel32Policy::Disabled;
    let access = host.begin_program(&input, config).unwrap();
    assert!(matches!(host.program_load_kernel32(&access, &input), Err(Stop::Unsupported(_))));
    assert!(host.program_lease_import(&access, &input, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert_eq!(history(&host).modules_issued, 0);
    assert_eq!(host.live_capabilities(), 0);
}

#[test]
fn actual_import_and_lookup_bindings_have_distinct_kinds_and_dispatch() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    assert_eq!(module, CAP_START);
    assert_eq!(calls, [CAP_START + 1, CAP_START + 2, CAP_START + 3, CAP_START + 4, CAP_START + 5]);
    assert!(matches!(host.capabilities.get(&module), Some(Capability::ProgramModule(_))));
    for call in calls {
        assert!(matches!(host.capabilities.get(&call), Some(Capability::ProgramImport(_))));
    }
    let weak_module = host.program_get_module_handle_a(&access, &input, calls[0], Some(b"KeRnEl32.DlL")).unwrap();
    let weak_alloc = host.program_get_proc_address(&access, &input, calls[1], weak_module, ProcedureQuery::Name(b"GlobalAlloc")).unwrap();
    assert!(matches!(host.capabilities.get(&weak_module), Some(Capability::ProgramModule(_))));
    assert!(matches!(host.capabilities.get(&weak_alloc), Some(Capability::ProgramProcedure(_))));
    assert!(weak_alloc > weak_module && weak_module > calls[4]);
    let pointer = host.program_global_alloc(&access, &input, weak_alloc, 0x40, 37).unwrap();
    assert_ne!(pointer, weak_alloc);
    assert_eq!(pointer % 8, 0);
    assert_eq!(host.memory.allocation_bytes(u64::from(pointer)).unwrap(), &[0; 37]);
    assert_eq!(host.memory.allocation_kind(u64::from(pointer)).unwrap(), super::super::Kind::ProgramGlobal);
    assert_eq!(host.program_global_free(&access, &input, calls[3], pointer).unwrap(), GlobalFreeReturn::Released);
    assert_eq!(host.memory.live_allocations(), 0);
}

#[test]
fn all_five_exports_dispatch_through_genuine_weak_procedures_without_import_upgrade() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, module) = begin(&mut host, &input);
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let weak_get_proc = host.program_get_proc_address(&access, &input, get_proc, module, ProcedureQuery::Name(b"GetProcAddress")).unwrap();
    let weak_get_module = host.program_get_proc_address(&access, &input, weak_get_proc, module, ProcedureQuery::Name(b"GetModuleHandleA")).unwrap();
    let weak_module = host.program_get_module_handle_a(&access, &input, weak_get_module, Some(b"kernel32")).unwrap();
    let [alloc, free, exit] = [Export::GlobalAlloc, Export::GlobalFree, Export::ExitProcess].map(|export| {
        host.program_get_proc_address(&access, &input, weak_get_proc, weak_module, ProcedureQuery::Name(export.name())).unwrap()
    });
    assert_eq!(history(&host).procedure_leases_issued, 1);
    for word in [weak_get_proc, weak_get_module, alloc, free, exit] {
        assert!(matches!(host.capabilities.get(&word), Some(Capability::ProgramProcedure(_))));
    }
    let pointer = host.program_global_alloc(&access, &input, alloc, 0, 11).unwrap();
    assert_eq!(host.program_global_free(&access, &input, free, pointer).unwrap(), GlobalFreeReturn::Released);
    let terminal: ProgramTermination = host.program_exit_process(&access, &input, exit, 9).unwrap();
    assert_eq!(terminal, ProgramTermination { exit_code: 9, released_globals: 0 });
}

#[test]
fn lookup_absence_case_and_ordinal_rules_come_from_real_backend() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    for name in [b"user32.dll".as_slice(), b"missing"] {
        let before = (state(&host), history(&host));
        assert_eq!(host.program_get_module_handle_a(&access, &input, calls[0], Some(name)).unwrap(), 0);
        assert_eq!((state(&host), history(&host)), before);
    }
    for query in [ProcedureQuery::Name(b"globalalloc"), ProcedureQuery::Name(b"AbsentExport")] {
        let before = (state(&host), history(&host));
        assert_eq!(host.program_get_proc_address(&access, &input, calls[1], module, query).unwrap(), 0);
        assert_eq!((state(&host), history(&host)), before);
    }
    for name in [None, Some(b"".as_slice()), Some(b"../kernel32"), Some(b"kernel32\0.dll"), Some(b"x\\kernel32")] {
        assert!(host.program_get_module_handle_a(&access, &input, calls[0], name).is_err());
    }
    for query in [ProcedureQuery::Ordinal(1), ProcedureQuery::Name(b"GlobalAlloc\0"), ProcedureQuery::Name(b"\xff")] {
        assert!(host.program_get_proc_address(&access, &input, calls[1], module, query).is_err());
        assert!(host.program_lease_import(&access, &input, b"kernel32", query).is_err());
    }
    for (library, name) in [(b"user32.dll".as_slice(), b"GlobalAlloc".as_slice()), (b"kernel32", b"globalalloc"), (b"kernel32", b"Missing")] {
        assert!(matches!(host.program_lease_import(&access, &input, library, ProcedureQuery::Name(name)), Err(Stop::Unsupported(_))));
    }
    assert_eq!(history(&host).procedure_leases_issued, 5);
}

#[test]
fn genuine_global_receipts_support_bytes_free_retry_and_never_adopt_world() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut world = host.memory.allocate_owner(32).unwrap();
    world.data.fill(0x5a);
    host.memory.synchronize_owner(&world).unwrap();
    let (access, _) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    let first = host.program_global_alloc(&access, &input, calls[2], 0, 19).unwrap();
    let second = host.program_global_alloc(&access, &input, calls[2], 0x40, 21).unwrap();
    for (pointer, bytes) in [(first, 19), (second, 21)] {
        assert_eq!(host.memory.allocation_bytes(u64::from(pointer)).unwrap(), vec![0; bytes]);
        let receipt = host.program.as_ref().unwrap().globals.iter().find(|r| r.pointer == pointer).unwrap();
        assert_eq!(host.program.as_ref().unwrap().registry.allocation_word(&host.memory, &receipt.owner).unwrap(), pointer);
    }
    let before = (state(&host), history(&host));
    for handle in [0, u32::MAX, first + 1, first - 1, world.token as u32] {
        assert!(host.program_global_free(&access, &input, calls[3], handle).is_err());
    }
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.program_global_free(&access, &input, calls[3], first).unwrap().return_word(), 0);
    assert!(host.memory.allocation_bytes(u64::from(first)).is_err());
    let repeated = host.program_global_free(&access, &input, calls[3], first).unwrap();
    assert_eq!(repeated, GlobalFreeReturn::Failed { original_handle: first, reason: FreeFailure::AlreadyReleased });
    assert_eq!(repeated.return_word(), first);
    assert_eq!(host.program.as_ref().unwrap().globals.len(), 2);
    let third = host.program_global_alloc(&access, &input, calls[2], 0, 8).unwrap();
    assert!(third > second && third != first);
    assert_eq!(host.memory.allocation_bytes(world.token).unwrap(), world.data);
}

#[test]
fn native_world_file_section_module_and_wrong_export_words_cannot_dispatch() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let world = host.memory.allocate_owner(32).unwrap();
    let path::Opened::File(file) = path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap() else { panic!() };
    let section = manager::Host::initialize_section(&mut host, 0x6fe578, 24).unwrap();
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    let pointer = host.program_global_alloc(&access, &input, calls[2], 0, 8).unwrap();
    let before = (state(&host), history(&host));
    for word in [0, 1, u32::MAX, world.token as u32, pointer, file as u32, section as u32, module] {
        assert!(host.program_get_module_handle_a(&access, &input, word, Some(b"kernel32")).is_err());
        assert!(host.program_get_proc_address(&access, &input, word, module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
        assert!(host.program_global_alloc(&access, &input, word, 0, 8).is_err());
        assert!(host.program_global_free(&access, &input, word, pointer).is_err());
        assert!(host.program_exit_process(&access, &input, word, 0).is_err());
    }
    for word in [0, world.token as u32, file as u32, section as u32, calls[0], calls[2]] {
        assert!(host.program_get_proc_address(&access, &input, calls[1], word, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    }
    for word in calls.into_iter().filter(|word| *word != calls[2]) {
        assert!(host.program_global_alloc(&access, &input, word, 0, 8).is_err());
    }
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.memory.allocation_bytes(world.token).unwrap(), world.data);
    assert_eq!(host.file(file as u32).unwrap().bytes, b"world bytes");
    assert!(host.section(section, 0x6fe578, 24).is_ok());
}

#[test]
fn same_words_in_foreign_services_do_not_confer_authority_even_with_same_input() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut first = fixture.services(8192);
    let mut second = fixture.services(8192);
    let (first_access, first_module) = begin(&mut first, &input);
    let (second_access, second_module) = begin(&mut second, &input);
    let first_calls = imports(&mut first, &first_access, &input);
    let second_calls = imports(&mut second, &second_access, &input);
    let first_global = first.program_global_alloc(&first_access, &input, first_calls[2], 0, 16).unwrap();
    let second_global = second.program_global_alloc(&second_access, &input, second_calls[2], 0, 16).unwrap();
    assert_eq!((first_module, first_calls, first_global), (second_module, second_calls, second_global));
    all_refused(&mut second, &first_access, &input, first_module, first_calls, first_global);
    all_refused(&mut first, &second_access, &input, second_module, second_calls, second_global);
    // Local access deliberately selects the local word; a word has no authority
    // independent of that exact service/access/Module context.
    assert_eq!(second.program_global_free(&second_access, &input, second_calls[3], second_global).unwrap(), GlobalFreeReturn::Released);
    assert_eq!(first.memory.allocation_bytes(u64::from(first_global)).unwrap(), &[0; 16]);
}

#[test]
fn same_byte_separate_reads_cannot_satisfy_the_retained_module_owner() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let reread = fixture.input();
    assert_eq!(input.image().raw(), reread.image().raw());
    assert_eq!(input.observation().sha256, reread.observation().sha256);
    assert_ne!(input.origin(), reread.origin());
    let mut host = fixture.services(8192);
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    let global = host.program_global_alloc(&access, &input, calls[2], 0, 8).unwrap();
    all_refused(&mut host, &access, &reread, module, calls, global);
    assert_eq!(host.program_global_free(&access, &input, calls[3], global).unwrap(), GlobalFreeReturn::Released);
}

#[test]
fn dropped_input_origin_observation_cannot_authorize_a_replacement_read() {
    let fixture = Fixture::new();
    let mut host = fixture.services(8192);
    let (access, module, calls, global, historical_origin) = {
        let input = fixture.input();
        let (access, module) = begin(&mut host, &input);
        let calls = imports(&mut host, &access, &input);
        let global = host.program_global_alloc(&access, &input, calls[2], 0, 8).unwrap();
        (access, module, calls, global, input.origin())
    };
    let replacement = fixture.input();
    assert_ne!(historical_origin, replacement.origin());
    all_refused(&mut host, &access, &replacement, module, calls, global);
    assert!(host.begin_program(&replacement, configuration()).is_err());
}

#[test]
fn dropped_service_access_cannot_authorize_reused_words_or_its_original_heap() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut old = fixture.services(8192);
    let (access, module) = begin(&mut old, &input);
    let calls = imports(&mut old, &access, &input);
    // Move the real heap to a fresh service to prove that heap equality alone
    // cannot replace the dropped service/program issuance identity.
    let heap = std::mem::replace(&mut old.memory, super::super::Memory::new(8192).unwrap());
    drop(old);
    let mut replacement = BootstrapServices::new(heap, &fixture.index, &fixture.directories, LocalePolicy::C);
    all_refused(&mut replacement, &access, &input, module, calls, 0);
    let (new_access, new_module) = begin(&mut replacement, &input);
    let new_calls = imports(&mut replacement, &new_access, &input);
    assert_eq!((module, calls), (new_module, new_calls));
    all_refused(&mut replacement, &access, &input, module, calls, 0);
    assert_ne!(replacement.program_global_alloc(&new_access, &input, new_calls[2], 0, 8).unwrap(), 0);
}

#[test]
fn heap_swap_rejects_all_operations_and_preserves_both_worlds() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut world = host.memory.allocate_owner(32).unwrap();
    world.data.fill(0x6a);
    host.memory.synchronize_owner(&world).unwrap();
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    let global = host.program_global_alloc(&access, &input, calls[2], 0, 16).unwrap();
    let mut foreign = super::super::Memory::new(8192).unwrap();
    let mut foreign_world = foreign.allocate_owner(32).unwrap();
    foreign_world.data.fill(0x7b);
    foreign.synchronize_owner(&foreign_world).unwrap();
    assert_eq!(world.token, foreign_world.token);
    let original = std::mem::replace(&mut host.memory, foreign);
    all_refused(&mut host, &access, &input, module, calls, global);
    assert!(host.begin_program(&input, configuration()).is_err());
    assert_eq!(host.memory.allocation_bytes(foreign_world.token).unwrap(), foreign_world.data);
    assert_eq!(original.allocation_bytes(world.token).unwrap(), world.data);
    assert_eq!(original.allocation_bytes(u64::from(global)).unwrap(), &[0; 16]);
    let foreign = std::mem::replace(&mut host.memory, original);
    assert_eq!(host.program_global_free(&access, &input, calls[3], global).unwrap(), GlobalFreeReturn::Released);
    assert_eq!(foreign.allocation_bytes(foreign_world.token).unwrap(), foreign_world.data);
}

#[test]
fn capability_budget_preflights_module_import_and_both_lookup_issuances() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut empty = fixture.services(8192);
    let access = empty.begin_program(&input, configuration()).unwrap();
    empty.next = CAP_START + MAX_CAPABILITIES;
    let before = (state(&empty), history(&empty));
    assert!(matches!(empty.program_load_kernel32(&access, &input), Err(Stop::Unavailable(_))));
    assert_eq!((state(&empty), history(&empty)), before);
    assert_eq!(history(&empty).modules_issued, 0);

    let mut host = fixture.services(8192);
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    host.next = CAP_START + MAX_CAPABILITIES;
    let before = (state(&host), history(&host));
    assert!(matches!(host.program_lease_import(&access, &input, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")), Err(Stop::Unavailable(_))));
    assert!(matches!(host.program_get_module_handle_a(&access, &input, calls[0], Some(b"kernel32")), Err(Stop::Unavailable(_))));
    assert!(matches!(host.program_get_proc_address(&access, &input, calls[1], module, ProcedureQuery::Name(b"GlobalAlloc")), Err(Stop::Unavailable(_))));
    assert_eq!((state(&host), history(&host)), before);
}

#[test]
fn bounded_freed_receipt_history_stops_before_new_allocation_and_keeps_retry() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut config = configuration();
    config.limits.global_allocation_issuances = 1;
    let access = host.begin_program(&input, config).unwrap();
    host.program_load_kernel32(&access, &input).unwrap();
    let calls = imports(&mut host, &access, &input);
    let pointer = host.program_global_alloc(&access, &input, calls[2], 0, 8).unwrap();
    assert_eq!(host.program_global_free(&access, &input, calls[3], pointer).unwrap(), GlobalFreeReturn::Released);
    let before = (state(&host), history(&host));
    assert!(matches!(host.program_global_alloc(&access, &input, calls[2], 0, 8), Err(Stop::Unavailable(_))));
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.program_global_free(&access, &input, calls[3], pointer).unwrap().return_word(), pointer);
    assert_eq!(history(&host).global_allocations_issued, 1);
    assert_eq!(host.memory.live_bytes(), 0);
}

#[test]
fn backend_lease_and_weak_budgets_refuse_without_publishing_capabilities() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut config = configuration();
    config.limits.procedure_lease_issuances = 2;
    config.limits.weak_observation_issuances = 1;
    let access = host.begin_program(&input, config).unwrap();
    let module = host.program_load_kernel32(&access, &input).unwrap();
    let get_module = lease(&mut host, &access, &input, Export::GetModuleHandleA);
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let before = (state(&host), history(&host));
    assert!(matches!(host.program_get_module_handle_a(&access, &input, get_module, Some(b"kernel32")), Err(Stop::Unavailable(_))));
    assert!(matches!(host.program_get_proc_address(&access, &input, get_proc, module, ProcedureQuery::Name(b"GlobalAlloc")), Err(Stop::Unavailable(_))));
    assert!(matches!(host.program_lease_import(&access, &input, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")), Err(Stop::Unavailable(_))));
    assert_eq!((state(&host), history(&host)), before);
    // Backend absence remains genuine even when its observation history is full.
    assert_eq!(host.program_get_proc_address(&access, &input, get_proc, module, ProcedureQuery::Name(b"Absent")).unwrap(), 0);
}

#[test]
fn heap_and_program_profile_failures_remain_errors_preserving_world_and_retry() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(64);
    let world = host.memory.allocate_owner(32).unwrap();
    let (access, _) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    let before = (state(&host), history(&host));
    assert!(matches!(host.program_global_alloc(&access, &input, calls[2], 0, 64), Err(Stop::Unavailable(_))));
    for (flags, bytes) in [(2, 8), (0, 0), (0, 1025)] {
        assert!(matches!(host.program_global_alloc(&access, &input, calls[2], flags, bytes), Err(Stop::Unsupported(_))));
    }
    assert_eq!((state(&host), history(&host)), before);
    let global = host.program_global_alloc(&access, &input, calls[2], 0, 16).unwrap();
    assert_ne!(global, 0);
    assert_eq!(host.memory.allocation_bytes(world.token).unwrap(), world.data);
    assert_eq!(history(&host).global_allocations_issued, 1);
}

#[test]
fn live_global_byte_profile_stops_before_effects_and_free_permits_a_fresh_retry() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let world = host.memory.allocate_owner(32).unwrap();
    let mut config = configuration();
    config.limits.max_live_global_bytes = 24;
    config.limits.max_global_bytes = 16;
    let access = host.begin_program(&input, config).unwrap();
    host.program_load_kernel32(&access, &input).unwrap();
    let calls = imports(&mut host, &access, &input);
    let first = host.program_global_alloc(&access, &input, calls[2], 0, 16).unwrap();
    let before = (state(&host), history(&host));
    assert!(matches!(host.program_global_alloc(&access, &input, calls[2], 0, 16), Err(Stop::Unavailable(_))));
    assert_eq!((state(&host), history(&host)), before);
    host.program_global_free(&access, &input, calls[3], first).unwrap();
    let retry = host.program_global_alloc(&access, &input, calls[2], 0, 16).unwrap();
    assert!(retry > first);
    assert_eq!(history(&host).global_allocations_issued, 2);
    assert_eq!(history(&host).live_global_bytes, 16);
    assert_eq!(host.memory.allocation_bytes(world.token).unwrap(), world.data);
}

#[test]
fn weak_capabilities_do_not_pin_or_upgrade_and_retired_words_never_revive() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, old_module) = begin(&mut host, &input);
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let old_alloc = host.program_get_proc_address(&access, &input, get_proc, old_module, ProcedureQuery::Name(b"GlobalAlloc")).unwrap();
    assert!(host.program.as_mut().unwrap().registry.retire_kernel32(&host.memory).is_err());
    // Private lifecycle fault injection: drop the actual owning import. This
    // service layer exposes no public raw-word removal or retirement API.
    assert!(matches!(host.capabilities.remove(&get_proc), Some(Capability::ProgramImport(_))));
    host.program.as_mut().unwrap().registry.retire_kernel32(&host.memory).unwrap();
    let before = state(&host);
    assert!(host.program_global_alloc(&access, &input, old_alloc, 0, 8).is_err());
    assert_eq!(state(&host), before);
    let new_module = host.program_load_kernel32(&access, &input).unwrap();
    assert!(new_module > old_alloc && new_module != old_module);
    let new_get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    assert!(host.program_get_proc_address(&access, &input, new_get_proc, old_module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert!(host.program_global_alloc(&access, &input, old_alloc, 0, 8).is_err());
    let new_alloc = host.program_get_proc_address(&access, &input, new_get_proc, new_module, ProcedureQuery::Name(b"GlobalAlloc")).unwrap();
    assert_ne!(host.program_global_alloc(&access, &input, new_alloc, 0, 8).unwrap(), 0);
}

#[test]
fn disabled_terminal_policy_refuses_even_through_genuine_weak_exit_lookup() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut config = configuration();
    config.exit = ExitPolicy::Disabled;
    let access = host.begin_program(&input, config).unwrap();
    let module = host.program_load_kernel32(&access, &input).unwrap();
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let alloc = lease(&mut host, &access, &input, Export::GlobalAlloc);
    let global = host.program_global_alloc(&access, &input, alloc, 0, 16).unwrap();
    assert!(host.program_lease_import(&access, &input, b"kernel32", ProcedureQuery::Name(b"ExitProcess")).is_err());
    let weak_exit = host.program_get_proc_address(&access, &input, get_proc, module, ProcedureQuery::Name(b"ExitProcess")).unwrap();
    let before = (state(&host), history(&host));
    assert!(matches!(host.program_exit_process(&access, &input, weak_exit, 7), Err(Stop::Unsupported(_))));
    assert_eq!((state(&host), history(&host)), before);
    assert_eq!(host.memory.allocation_bytes(u64::from(global)).unwrap(), &[0; 16]);
}

#[test]
fn typed_terminal_finishes_once_releases_only_program_globals_and_seals_every_api() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let mut world = host.memory.allocate_owner(32).unwrap();
    world.data.fill(0x39);
    host.memory.synchronize_owner(&world).unwrap();
    let path::Opened::File(file) = path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap() else { panic!() };
    let section = manager::Host::initialize_section(&mut host, 0x6fe578, 24).unwrap();
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    let weak_exit = host.program_get_proc_address(&access, &input, calls[1], module, ProcedureQuery::Name(b"ExitProcess")).unwrap();
    let first = host.program_global_alloc(&access, &input, calls[2], 0, 8).unwrap();
    let second = host.program_global_alloc(&access, &input, calls[2], 0, 16).unwrap();
    let third = host.program_global_alloc(&access, &input, calls[2], 0x40, 24).unwrap();
    host.program_global_free(&access, &input, calls[3], first).unwrap();
    let next = host.next;
    let terminal: ProgramTermination = host.program_exit_process(&access, &input, weak_exit, 0x1234_5678).unwrap();
    assert_eq!(terminal, ProgramTermination { exit_code: 0x1234_5678, released_globals: 2 });
    assert!(host.program.as_ref().unwrap().sealed);
    assert!(host.program.as_ref().unwrap().registry.history(&host.memory).is_err());
    assert!(host.program.as_ref().unwrap().globals.is_empty());
    assert_eq!(host.next, next);
    assert_eq!(host.live_capabilities(), 2);
    for word in std::iter::once(module).chain(calls).chain(std::iter::once(weak_exit)) {
        assert!(!host.capabilities.contains_key(&word));
    }
    for pointer in [first, second, third] {
        assert!(host.memory.allocation_bytes(u64::from(pointer)).is_err());
    }
    assert_eq!(host.memory.live_bytes(), 32);
    assert_eq!(host.memory.allocation_bytes(world.token).unwrap(), world.data);
    assert_eq!(host.file(file as u32).unwrap().bytes, b"world bytes");
    assert!(host.section(section, 0x6fe578, 24).is_ok());
    all_refused(&mut host, &access, &input, module, calls, second);
    assert!(host.begin_program(&input, configuration()).is_err());
    // Unrelated capabilities remain usable and new shared capability words
    // continue monotonically instead of reviving any retired program word.
    let path::Opened::File(later_file) = path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap() else { panic!() };
    assert_eq!(later_file, u64::from(next));
    path::Host::close_scenario(&mut host, file).unwrap();
}

#[test]
fn terminal_preflight_failure_keeps_unrelated_world_and_program_bindings_live() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let world = host.memory.allocate_owner(32).unwrap();
    let (access, module) = begin(&mut host, &input);
    let calls = imports(&mut host, &access, &input);
    let first = host.program_global_alloc(&access, &input, calls[2], 0, 8).unwrap();
    let second = host.program_global_alloc(&access, &input, calls[2], 0, 16).unwrap();
    // Private fault injection through real Memory: externally missing storage
    // must stop terminal cleanup before it releases the other live allocation.
    host.memory.release(u64::from(second), super::super::Kind::ProgramGlobal).unwrap();
    let before = (state(&host), history(&host));
    assert!(host.program_exit_process(&access, &input, calls[4], 7).is_err());
    assert_eq!((state(&host), history(&host)), before);
    assert!(!host.program.as_ref().unwrap().sealed);
    assert_eq!(host.memory.allocation_bytes(u64::from(first)).unwrap(), &[0; 8]);
    assert_eq!(host.memory.allocation_bytes(world.token).unwrap(), world.data);
    assert_ne!(host.program_get_proc_address(&access, &input, calls[1], module, ProcedureQuery::Name(b"GlobalFree")).unwrap(), 0);
}

// Parent-authored real-file/service wiring controls. These exercise the actual
// executable reader, ProgramAccess, sealed procedures and owning store together.
fn private_registry_limits() -> program_registry_store::Limits {
    program_registry_store::Limits { max_bytes: 1 << 20, max_keys: 128, max_records: 128 }
}
fn private_registry_hive(fixture: &Fixture, leaf: &str) -> program_registry_store::Hive {
    program_registry_store::Hive::create_new(&fixture.directory.canonicalize().unwrap(), std::ffi::OsStr::new(leaf),
                                           private_registry_limits()).unwrap()
}
fn private_registry_begin(host: &mut BootstrapServices<'_>, input: &Module,
                          hive: program_registry_store::Hive, max_handles: usize)
    -> (ProgramAccess, u32, u32, u32)
{
    let access = match host.begin_program_with_private_user_registry(input, configuration(),
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled, hive, max_handles)
    {
        Ok(access) => access,
        Err(failure) => panic!("actual private hive registration refused: {:?}", failure.error),
    };
    let module = host.program_load_module(&access, input, ModuleKind::Advapi32).unwrap();
    let create = host.program_lease_import(&access, input, b"ADVAPI32.dll",
        ProcedureQuery::Name(b"RegCreateKeyA")).unwrap();
    let close = host.program_lease_import(&access, input, b"advapi32",
        ProcedureQuery::Name(b"RegCloseKey")).unwrap();
    (access, module, create, close)
}
fn private_registry_create(host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module,
                           create: u32, parent: u32, bytes: &[u8]) -> Result<u32, Stop> {
    let text = std::ffi::CString::new(bytes).unwrap();
    host.program_reg_create_key_a(access, input, create, parent, program_registry::SubKey::Ansi(&text))
}
fn private_registry_root() -> u32 { program_registry::PredefinedRoot::CurrentUser.word() }

#[test]
fn registry_binding_real_keys_persist_and_duplicate_handles_close_independently() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "persistent.hive"), 16);
    let first = private_registry_create(&mut host, &access, &input, create, private_registry_root(),
        b"Software\\Sims").unwrap();
    let committed = host.program_private_registry_observation(&access, &input).unwrap();
    assert_eq!((committed.keys, committed.records), (3, 1));
    let second = private_registry_create(&mut host, &access, &input, create, private_registry_root(),
        b"sOFTWARE\\sIMS").unwrap();
    assert_ne!(first, second);
    assert_eq!(host.program_private_registry_observation(&access, &input).unwrap(), committed);
    host.program_reg_close_key(&access, &input, close, first).unwrap();
    assert!(host.program_reg_close_key(&access, &input, close, first).is_err());
    assert!(private_registry_create(&mut host, &access, &input, create, first, b"Rejected").is_err());
    let child = private_registry_create(&mut host, &access, &input, create, second, b"Households").unwrap();
    host.program_reg_close_key(&access, &input, close, second).unwrap();
    host.program_reg_close_key(&access, &input, close, child).unwrap();
    let final_state = host.program_private_registry_observation(&access, &input).unwrap();
    assert_eq!((final_state.keys, final_state.records), (4, 2));
    drop(host);
    let mut reopened = program_registry_store::Hive::open(&fixture.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("persistent.hive"), private_registry_limits()).unwrap();
    assert_eq!(reopened.snapshot(), final_state);
    let root = reopened.root().unwrap();
    let existing = reopened.create(&root, b"SOFTWARE\\SIMS\\HOUSEHOLDS").unwrap();
    assert_eq!(existing.canonical_ascii(), b"software\\sims\\households");
    assert_eq!(reopened.snapshot(), final_state);
}

#[test]
fn registry_binding_foreign_program_input_procedure_and_heap_refuse_before_writes() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let same_bytes = fixture.input();
    let mut host = fixture.services(8192);
    let (access, module, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "guarded.hive"), 16);
    let mut foreign = fixture.services(8192);
    let (other_access, _, other_create, _) = private_registry_begin(&mut foreign, &input,
        private_registry_hive(&fixture, "foreign.hive"), 16);
    assert_eq!(create, other_create); // Repeated words confer no cross-service authority.
    host.program_load_kernel32(&access, &input).unwrap();
    let kernel = lease(&mut host, &access, &input, Export::GlobalAlloc);
    let before = (state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                  fs::read(fixture.directory.join("guarded.hive")).unwrap());
    assert!(private_registry_create(&mut host, &other_access, &input, create, private_registry_root(), b"Bad").is_err());
    assert!(private_registry_create(&mut foreign, &access, &input, other_create, private_registry_root(), b"Bad").is_err());
    assert!(private_registry_create(&mut host, &access, &same_bytes, create, private_registry_root(), b"Bad").is_err());
    for wrong in [0, u32::MAX, module, close, kernel] {
        assert!(private_registry_create(&mut host, &access, &input, wrong, private_registry_root(), b"Bad").is_err());
    }
    for wrong_parent in [0, module, create, kernel] {
        assert!(private_registry_create(&mut host, &access, &input, create, wrong_parent, b"Bad").is_err());
    }
    let original_heap = std::mem::replace(&mut host.memory, Memory::new(8192).unwrap());
    assert!(private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Bad").is_err());
    assert!(host.program_private_registry_observation(&access, &input).is_err());
    host.memory = original_heap;
    assert_eq!((state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                fs::read(fixture.directory.join("guarded.hive")).unwrap()), before);
}

#[test]
fn registry_binding_capability_budget_precedes_disk_mutation_but_null_roots_allocate_nothing() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, _) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "budget.hive"), 16);
    host.next = CAP_START + MAX_CAPABILITIES;
    let bytes = fs::read(fixture.directory.join("budget.hive")).unwrap();
    let before = (state(&host), host.program_private_registry_observation(&access, &input).unwrap());
    assert!(private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"NoWrite").is_err());
    for root in [program_registry::PredefinedRoot::ClassesRoot, program_registry::PredefinedRoot::CurrentUser,
        program_registry::PredefinedRoot::LocalMachine, program_registry::PredefinedRoot::Users,
        program_registry::PredefinedRoot::CurrentConfig]
    {
        let word = root.word();
        assert_eq!(host.program_reg_create_key_a(&access, &input, create, word,
            program_registry::SubKey::Null).unwrap(), word);
    }
    assert_eq!((state(&host), host.program_private_registry_observation(&access, &input).unwrap()), before);
    assert_eq!(fs::read(fixture.directory.join("budget.hive")).unwrap(), bytes);
}

#[test]
fn registry_binding_handle_history_is_not_recycled_after_close() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "handles.hive"), 2);
    let first = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"First").unwrap();
    let second = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Second").unwrap();
    host.program_reg_close_key(&access, &input, close, first).unwrap();
    host.program_reg_close_key(&access, &input, close, second).unwrap();
    let before = (state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                  fs::read(fixture.directory.join("handles.hive")).unwrap());
    assert!(private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Third").is_err());
    assert_eq!((state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                fs::read(fixture.directory.join("handles.hive")).unwrap()), before);
}

#[test]
fn registry_binding_unsupported_roots_text_null_owned_and_wrong_close_are_effect_free() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, module, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "unsupported.hive"), 16);
    let key = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Live").unwrap();
    let before = (state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                  fs::read(fixture.directory.join("unsupported.hive")).unwrap());
    for root in [program_registry::PredefinedRoot::ClassesRoot, program_registry::PredefinedRoot::LocalMachine,
        program_registry::PredefinedRoot::Users, program_registry::PredefinedRoot::CurrentConfig]
    {
        assert!(private_registry_create(&mut host, &access, &input, create, root.word(), b"Bad").is_err());
    }
    for name in [b"".as_slice(), b"\xff", b"\\Bad", b"Bad\\", b"Bad\\\\Path"] {
        assert!(private_registry_create(&mut host, &access, &input, create, private_registry_root(), name).is_err());
    }
    assert!(host.program_reg_create_key_a(&access, &input, create, key, program_registry::SubKey::Null).is_err());
    for bad in [private_registry_root(), module, create, u32::MAX] {
        assert!(host.program_reg_close_key(&access, &input, close, bad).is_err());
    }
    assert!(host.program_reg_close_key(&access, &input, create, key).is_err());
    assert_eq!((state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                fs::read(fixture.directory.join("unsupported.hive")).unwrap()), before);
    host.program_reg_close_key(&access, &input, close, key).unwrap();
}

#[test]
fn registry_binding_old_constructors_disable_and_reconfiguration_returns_locked_hive() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let access = host.begin_program_with_profiles(&input, configuration(), program_ole32::Policy::OwnedRustSta,
        program_winmm::Policy::Disabled).unwrap();
    assert!(host.program_load_module(&access, &input, ModuleKind::Advapi32).is_err());
    let failure = match host.begin_program_with_private_user_registry(&input, configuration(),
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled,
        private_registry_hive(&fixture, "retained.hive"), 16)
    {
        Err(failure) => failure,
        Ok(_) => panic!("registered program must never be replaced"),
    };
    assert!(program_registry_store::Hive::open(&fixture.directory.canonicalize().unwrap(), std::ffi::OsStr::new("retained.hive"),
        private_registry_limits()).is_err());
    assert_eq!(failure.hive.snapshot().keys, 1);
    assert!(host.program_load_module(&access, &input, ModuleKind::Advapi32).is_err());
    drop(failure);
    let reopened = program_registry_store::Hive::open(&fixture.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("retained.hive"), private_registry_limits()).unwrap();
    assert_eq!(reopened.snapshot().keys, 1);
}

#[test]
fn registry_binding_begin_refusal_returns_same_hive_and_allows_explicit_valid_registration() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let hive = private_registry_hive(&fixture, "begin.hive");
    let before = fs::read(fixture.directory.join("begin.hive")).unwrap();
    let failure = match host.begin_program_with_private_user_registry(&input, configuration(),
        program_ole32::Policy::Disabled, program_winmm::Policy::Disabled, hive, 0)
    {
        Err(failure) => failure,
        Ok(_) => panic!("zero handle history must refuse"),
    };
    assert!(host.program.is_none());
    assert_eq!(fs::read(fixture.directory.join("begin.hive")).unwrap(), before);
    assert!(program_registry_store::Hive::open(&fixture.directory.canonicalize().unwrap(), std::ffi::OsStr::new("begin.hive"),
        private_registry_limits()).is_err());
    let (access, _, create, _) = private_registry_begin(&mut host, &input, failure.hive, 8);
    private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Actual").unwrap();
    assert_eq!(host.program_private_registry_observation(&access, &input).unwrap().keys, 2);
}

#[test]
fn registry_binding_table_reload_preserves_keys_and_old_table_identity_is_revoked() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, module, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "reload.hive"), 16);
    let identity = host.program_module_identity(&access, &input, module).unwrap();
    let key = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Persistent").unwrap();
    assert!(host.program.as_mut().unwrap().registry.retire_module(&host.memory, ModuleKind::Advapi32).is_err());
    host.capabilities.remove(&create);
    host.capabilities.remove(&close);
    host.program.as_mut().unwrap().registry.retire_module(&host.memory, ModuleKind::Advapi32).unwrap();
    assert!(host.program_module_identity(&access, &input, module).is_err());
    let replacement = host.program_load_module(&access, &input, ModuleKind::Advapi32).unwrap();
    assert_ne!(host.program_module_identity(&access, &input, replacement).unwrap(), identity);
    let close = host.program_lease_import(&access, &input, b"advapi32", ProcedureQuery::Name(b"RegCloseKey")).unwrap();
    host.program_reg_close_key(&access, &input, close, key).unwrap();
    assert_eq!(host.program_private_registry_observation(&access, &input).unwrap().keys, 2);
}

#[test]
fn registry_binding_terminal_program_revokes_keys_and_releases_store_without_deleting_keys() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "terminal.hive"), 16);
    let key = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Survives").unwrap();
    host.program_load_kernel32(&access, &input).unwrap();
    let exit = lease(&mut host, &access, &input, Export::ExitProcess);
    let termination = host.program_exit_process(&access, &input, exit, 7).unwrap();
    assert_eq!(termination.exit_code, 7);
    assert!(host.program_reg_close_key(&access, &input, close, key).is_err());
    assert!(private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Bad").is_err());
    assert!(host.capabilities.values().all(|value| !matches!(value, Capability::ProgramRegistryKey(_))));
    let reopened = program_registry_store::Hive::open(&fixture.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("terminal.hive"), private_registry_limits()).unwrap();
    assert_eq!((reopened.snapshot().keys, reopened.snapshot().records), (2, 1));
}

#[test]
fn registry_binding_genuine_mixed_file_and_key_issuances_exhaust_shared_history() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "mixed-budget.hive"), 4096);
    assert_eq!(host.next, CAP_START + 3);
    for _ in 0..2046 {
        assert!(matches!(path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap(),
                         path::Opened::File(_)));
        let key = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Existing").unwrap();
        host.program_reg_close_key(&access, &input, close, key).unwrap();
    }
    assert!(matches!(path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap(),
                     path::Opened::File(_)));
    assert_eq!(host.next, CAP_START + MAX_CAPABILITIES);
    let before = (state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                  fs::read(fixture.directory.join("mixed-budget.hive")).unwrap());
    assert!(private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"NoWrite").is_err());
    assert!(path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").is_err());
    assert_eq!(host.program_reg_create_key_a(&access, &input, create, private_registry_root(),
        program_registry::SubKey::Null).unwrap(), private_registry_root());
    assert_eq!((state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                fs::read(fixture.directory.join("mixed-budget.hive")).unwrap()), before);
}

#[test]
fn registry_binding_weak_procedures_dispatch_then_refuse_after_exact_table_replacement() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, module, create_import, close_import) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "weak-reload.hive"), 16);
    host.program_load_kernel32(&access, &input).unwrap();
    let get_module = lease(&mut host, &access, &input, Export::GetModuleHandleA);
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let looked_up = host.program_get_module_handle_a(&access, &input, get_module, Some(b"ADVAPI32.dll")).unwrap();
    let create = host.program_get_proc_address(&access, &input, get_proc, looked_up,
        ProcedureQuery::Name(b"RegCreateKeyA")).unwrap();
    let close = host.program_get_proc_address(&access, &input, get_proc, looked_up,
        ProcedureQuery::Name(b"RegCloseKey")).unwrap();
    let key = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Persistent").unwrap();
    host.capabilities.remove(&create_import); host.capabilities.remove(&close_import);
    host.program.as_mut().unwrap().registry.retire_module(&host.memory, ModuleKind::Advapi32).unwrap();
    let replacement = host.program_load_module(&access, &input, ModuleKind::Advapi32).unwrap();
    assert_ne!(module, replacement);
    let before = (state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                  fs::read(fixture.directory.join("weak-reload.hive")).unwrap());
    assert!(private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Rejected").is_err());
    assert!(host.program_reg_close_key(&access, &input, close, key).is_err());
    assert!(host.program_get_proc_address(&access, &input, get_proc, looked_up,
        ProcedureQuery::Name(b"RegCreateKeyA")).is_err());
    assert_eq!((state(&host), host.program_private_registry_observation(&access, &input).unwrap(),
                fs::read(fixture.directory.join("weak-reload.hive")).unwrap()), before);
    let new_close = host.program_get_proc_address(&access, &input, get_proc, replacement,
        ProcedureQuery::Name(b"RegCloseKey")).unwrap();
    host.program_reg_close_key(&access, &input, new_close, key).unwrap();
}

// Registry output consumer controls use real descriptor
// inputs, private file hives, issuing registries and GlobalAlloc/GlobalFree.
// They do not qualify guest ABI/LSTATUS/fault handling or native execution.
fn registry_output_imports(host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module)
    -> (u32, u32)
{
    host.program_load_kernel32(access, input).unwrap();
    (lease(host, access, input, Export::GlobalAlloc), lease(host, access, input, Export::GlobalFree))
}

fn registry_output_buffer(host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module,
                          alloc: u32, bytes: u32) -> (u32, Vec<u8>) {
    let pointer = host.program_global_alloc(access, input, alloc, 0, bytes).unwrap();
    let program = host.program.as_ref().unwrap();
    let receipt = program.globals.iter().find(|r| r.pointer == pointer).unwrap();
    assert_eq!(program.registry.allocation_word(&host.memory, &receipt.owner).unwrap(), pointer);
    let mut data = host.memory.allocation_bytes(u64::from(pointer)).unwrap().to_vec();
    for (i, byte) in data.iter_mut().enumerate() { *byte = 0x51u8.wrapping_add(i as u8); }
    // An image initializes test bytes only after genuine receipt validation.
    host.memory.synchronize_owner(&super::super::Owner { token: u64::from(pointer), data: data.clone() }).unwrap();
    (pointer, data)
}

fn registry_output_create(host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module,
                          create: u32, parent: u32, bytes: &[u8], output: u32) -> Result<u32, Stop> {
    let text = std::ffi::CString::new(bytes).unwrap();
    host.program_reg_create_key_a_into(access, input, create, parent, program_registry::SubKey::Ansi(&text), output)
}

fn registry_output_assert_independent_lock(path: &std::path::Path, busy: bool) {
    let file = OpenOptions::new().read(true).write(true).open(path).unwrap();
    // SAFETY: this independently opened descriptor owns the exact fixture file;
    // flock is nonblocking and its lock, if acquired, ends with this File.
    let result = unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_EX | libc::LOCK_NB) };
    if busy {
        let error = std::io::Error::last_os_error();
        assert_eq!(result, -1, "retained poisoned owner released its lock");
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    } else {
        assert_eq!(result, 0, "dropped owner retained its lock");
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RegistryOutputCheckpoint {
    service: (u32, usize, usize, usize, usize),
    modules: History,
    store: program_registry_store::Observation,
    journal: Vec<u8>,
    memory: Vec<(u32, Vec<u8>)>,
}

fn registry_output_checkpoint(fixture: &Fixture, host: &BootstrapServices<'_>, access: &ProgramAccess,
                              input: &Module, leaf: &str, pointers: &[u32]) -> RegistryOutputCheckpoint {
    RegistryOutputCheckpoint {
        service: state(host), modules: history(host),
        store: host.program_private_registry_observation(access, input).unwrap(),
        journal: fs::read(fixture.directory.join(leaf)).unwrap(),
        memory: pointers.iter().map(|p| (*p, host.memory.allocation_bytes(u64::from(*p)).unwrap().to_vec())).collect(),
    }
}

#[test]
fn registry_output_real_create_open_unaligned_store_preserves_all_other_bytes() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-real.hive"), 16);
    let (alloc, _) = registry_output_imports(&mut host, &access, &input);
    let (output, mut expected) = registry_output_buffer(&mut host, &access, &input, alloc, 19);
    let (other, untouched) = registry_output_buffer(&mut host, &access, &input, alloc, 23);
    let initial = fs::read(fixture.directory.join("output-real.hive")).unwrap();
    let first = registry_output_create(&mut host, &access, &input, create, private_registry_root(),
        b"Software\\Sims", output + 3).unwrap();
    assert_ne!(first, 0); // The written value is an actual HKEY, not LSTATUS0.
    assert!(matches!(host.capabilities.get(&first), Some(Capability::ProgramRegistryKey(_))));
    expected[3..7].copy_from_slice(&first.to_le_bytes());
    assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
    let committed = host.program_private_registry_observation(&access, &input).unwrap();
    assert_eq!((committed.keys, committed.records), (3, 1));
    let journal = fs::read(fixture.directory.join("output-real.hive")).unwrap();
    assert_ne!(journal, initial);
    let second = registry_output_create(&mut host, &access, &input, create, private_registry_root(),
        b"sOFTWARE\\sIMS", output + 15).unwrap(); // exact end, also unaligned
    assert_ne!(first, second);
    expected[15..19].copy_from_slice(&second.to_le_bytes());
    assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
    assert_eq!(host.memory.allocation_bytes(u64::from(other)).unwrap(), untouched);
    assert_eq!(host.program_private_registry_observation(&access, &input).unwrap(), committed);
    assert_eq!(fs::read(fixture.directory.join("output-real.hive")).unwrap(), journal);
    host.program_reg_close_key(&access, &input, close, first).unwrap();
    // The independently issued live key really authorizes an owned-parent call.
    let child = registry_output_create(&mut host, &access, &input, create, second, b"Households", output + 1).unwrap();
    expected[1..5].copy_from_slice(&child.to_le_bytes());
    assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
    assert_eq!(host.memory.allocation_bytes(u64::from(other)).unwrap(), untouched);
    host.program_reg_close_key(&access, &input, close, second).unwrap();
    host.program_reg_close_key(&access, &input, close, child).unwrap();
    let final_store = host.program_private_registry_observation(&access, &input).unwrap();
    assert_eq!((final_store.keys, final_store.records), (4, 2));
    drop(host);
    let mut reopened = program_registry_store::Hive::open(&fixture.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("output-real.hive"), private_registry_limits()).unwrap();
    assert_eq!(reopened.snapshot(), final_store);
    let root = reopened.root().unwrap();
    assert_eq!(reopened.create(&root, b"SOFTWARE\\SIMS\\HOUSEHOLDS").unwrap().canonical_ascii(),
        b"software\\sims\\households");
    assert_eq!(reopened.snapshot(), final_store);
}

#[test]
fn registry_output_borrowed_null_roots_write_real_words_without_key_issuance() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, _) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-roots.hive"), 16);
    let (alloc, _) = registry_output_imports(&mut host, &access, &input);
    let (output, mut expected) = registry_output_buffer(&mut host, &access, &input, alloc, 9);
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-roots.hive", &[]);
    for root in [program_registry::PredefinedRoot::ClassesRoot, program_registry::PredefinedRoot::CurrentUser,
        program_registry::PredefinedRoot::LocalMachine, program_registry::PredefinedRoot::Users,
        program_registry::PredefinedRoot::CurrentConfig]
    {
        let word = root.word();
        assert_eq!(host.program_reg_create_key_a_into(&access, &input, create, word,
            program_registry::SubKey::Null, output + 1).unwrap(), word);
        expected[1..5].copy_from_slice(&word.to_le_bytes());
        assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
        assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-roots.hive", &[]), before);
        assert!(!host.capabilities.contains_key(&word));
    }
}

#[test]
fn registry_output_invalid_cross_boundary_world_unreceipted_and_freed_spans_precede_key_effects() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, _) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-spans.hive"), 16);
    let (alloc, free) = registry_output_imports(&mut host, &access, &input);
    let (first, _) = registry_output_buffer(&mut host, &access, &input, alloc, 16);
    let (second, _) = registry_output_buffer(&mut host, &access, &input, alloc, 16);
    assert_eq!(second, first + 16); // Two real adjacent allocations cannot be joined.
    let (small, _) = registry_output_buffer(&mut host, &access, &input, alloc, 3);
    let (freed, _) = registry_output_buffer(&mut host, &access, &input, alloc, 16);
    assert_eq!(host.program_global_free(&access, &input, free, freed).unwrap(), GlobalFreeReturn::Released);
    assert!(host.memory.allocation_bytes(u64::from(freed)).is_err());
    let world = host.memory.allocate_owner(16).unwrap();
    // Genuine Memory allocation kind without this program's GlobalAllocation
    // receipt is deliberately insufficient, even though the bytes are live.
    let sims_sim::saved_matrix::Allocated::Bytes { token: raw, .. } = host.memory.allocate_program_global(16).unwrap()
        else { panic!("actual typed allocation must produce storage"); };
    let raw = u32::try_from(raw).unwrap();
    let pointers = [first, second, small, world.token as u32, raw];
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-spans.hive", &pointers);
    for output in [0, 1, u32::MAX, u32::MAX - 2, first - 1, first + 13, first + 14,
        first + 15, second + 16, small, freed, freed + 1, world.token as u32, world.token as u32 + 3, raw, raw + 1]
    {
        assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(),
            b"MustNotExist", output).is_err(), "unexpected acceptance of {output:#x}");
        assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-spans.hive", &pointers), before);
    }
    assert_eq!(host.memory.allocation_kind(u64::from(raw)).unwrap(), super::super::Kind::ProgramGlobal);
    assert!(!host.program.as_ref().unwrap().globals.iter().any(|r| r.pointer == raw));
}

#[test]
fn registry_output_foreign_access_input_procedure_parent_and_real_heap_swap_preserve_preimages() {
    let fixture = Fixture::new(); let input = fixture.input(); let same_bytes = fixture.input();
    let mut host = fixture.services(8192); let mut foreign = fixture.services(8192);
    let (access, module, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-guarded.hive"), 16);
    let (other_access, _, other_create, _) = private_registry_begin(&mut foreign, &input,
        private_registry_hive(&fixture, "output-foreign.hive"), 16);
    let (alloc, free) = registry_output_imports(&mut host, &access, &input);
    let (other_alloc, _) = registry_output_imports(&mut foreign, &other_access, &input);
    let (output, _) = registry_output_buffer(&mut host, &access, &input, alloc, 16);
    let (other_output, _) = registry_output_buffer(&mut foreign, &other_access, &input, other_alloc, 16);
    assert_eq!((output, create), (other_output, other_create));
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-guarded.hive", &[output]);
    let foreign_before = registry_output_checkpoint(&fixture, &foreign, &other_access, &input, "output-foreign.hive", &[other_output]);
    assert!(registry_output_create(&mut host, &other_access, &input, create, private_registry_root(), b"Bad", output).is_err());
    assert!(registry_output_create(&mut foreign, &access, &input, other_create, private_registry_root(), b"Bad", other_output).is_err());
    assert!(registry_output_create(&mut host, &access, &same_bytes, create, private_registry_root(), b"Bad", output).is_err());
    for wrong in [0, u32::MAX, module, close, alloc, free] {
        assert!(registry_output_create(&mut host, &access, &input, wrong, private_registry_root(), b"Bad", output).is_err());
    }
    for parent in [0, u32::MAX, module, create, alloc, output] {
        assert!(registry_output_create(&mut host, &access, &input, create, parent, b"Bad", output).is_err());
    }
    // Use the other actual populated heap, including the repeating output word.
    std::mem::swap(&mut host.memory, &mut foreign.memory);
    assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(), b"Bad", output).is_err());
    assert!(registry_output_create(&mut foreign, &other_access, &input, other_create, private_registry_root(), b"Bad", other_output).is_err());
    std::mem::swap(&mut host.memory, &mut foreign.memory);
    assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-guarded.hive", &[output]), before);
    assert_eq!(registry_output_checkpoint(&fixture, &foreign, &other_access, &input, "output-foreign.hive", &[other_output]), foreign_before);
}

#[test]
fn registry_output_same_word_foreign_genuine_receipt_is_not_current_program_ownership() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192); let mut foreign = fixture.services(8192);
    let (access, _, create, _) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-receipt.hive"), 16);
    let (other_access, _, other_create, _) = private_registry_begin(&mut foreign, &input,
        private_registry_hive(&fixture, "output-other-receipt.hive"), 16);
    let (alloc, _) = registry_output_imports(&mut host, &access, &input);
    let (other_alloc, _) = registry_output_imports(&mut foreign, &other_access, &input);
    let (output, _) = registry_output_buffer(&mut host, &access, &input, alloc, 16);
    let (other_output, _) = registry_output_buffer(&mut foreign, &other_access, &input, other_alloc, 16);
    assert_eq!(output, other_output);
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-receipt.hive", &[output]);
    let other_before = registry_output_checkpoint(&fixture, &foreign, &other_access, &input, "output-other-receipt.hive", &[other_output]);
    // Exchange actual receipts, without constructing/adopting authority. Both
    // raw words and kinds still match; exact issuing-registry provenance fails.
    std::mem::swap(&mut host.program.as_mut().unwrap().globals[0].owner,
                   &mut foreign.program.as_mut().unwrap().globals[0].owner);
    assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(), b"Bad", output).is_err());
    assert!(registry_output_create(&mut foreign, &other_access, &input, other_create, private_registry_root(), b"Bad", other_output).is_err());
    assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-receipt.hive", &[output]), before);
    assert_eq!(registry_output_checkpoint(&fixture, &foreign, &other_access, &input, "output-other-receipt.hive", &[other_output]), other_before);
}

#[test]
fn registry_output_real_store_budget_exhaustion_preserves_output() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let hive = program_registry_store::Hive::create_new(&fixture.directory.canonicalize().unwrap(),
        std::ffi::OsStr::new("output-refusal.hive"),
        program_registry_store::Limits { max_keys: 2, max_records: 1, ..private_registry_limits() }).unwrap();
    let (access, _, create, close) = private_registry_begin(&mut host, &input, hive, 16);
    let (alloc, _) = registry_output_imports(&mut host, &access, &input);
    let (output, _) = registry_output_buffer(&mut host, &access, &input, alloc, 17);
    let key = registry_output_create(&mut host, &access, &input, create, private_registry_root(), b"Existing", output + 1).unwrap();
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-refusal.hive", &[output]);
    assert_eq!((before.store.keys, before.store.records), (2, 1));
    // This valid supported request tests exhausted budgets only. Input refusal
    // is exercised against a separate healthy store below.
    assert!(matches!(registry_output_create(&mut host, &access, &input, create, private_registry_root(),
        b"NewKey", output + 1), Err(Stop::Unsupported(_))));
    assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-refusal.hive", &[output]), before);
    assert!(!before.store.poisoned);
    host.program_reg_close_key(&access, &input, close, key).unwrap();
}

#[test]
fn registry_output_real_shared_capability_exhaustion_preserves_output_but_borrowed_root_can_store() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, _) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-budget.hive"), 16);
    let (alloc, _) = registry_output_imports(&mut host, &access, &input);
    let (output, mut expected) = registry_output_buffer(&mut host, &access, &input, alloc, 9);
    // Exhaust through genuine shared file capabilities, never a counter patch.
    for _ in host.next..CAP_START + MAX_CAPABILITIES {
        assert!(matches!(path::Host::open_scenario(&mut host, b"./GameData/Scenario.tds", b"rb").unwrap(),
                         path::Opened::File(_)));
    }
    assert_eq!(host.next, CAP_START + MAX_CAPABILITIES);
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-budget.hive", &[output]);
    assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(), b"NoWrite", output + 1).is_err());
    assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-budget.hive", &[output]), before);
    assert_eq!(host.program_reg_create_key_a_into(&access, &input, create, private_registry_root(),
        program_registry::SubKey::Null, output + 1).unwrap(), private_registry_root());
    expected[1..5].copy_from_slice(&private_registry_root().to_le_bytes());
    assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
    let after = registry_output_checkpoint(&fixture, &host, &access, &input, "output-budget.hive", &[output]);
    assert_eq!(after.service, before.service);
    assert_eq!(after.modules, before.modules);
    assert_eq!(after.store, before.store);
    assert_eq!(after.journal, before.journal);
}

#[test]
fn registry_output_real_damaged_then_poisoned_store_retains_owner_and_output() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-damaged.hive"), 16);
    let (alloc, _) = registry_output_imports(&mut host, &access, &input);
    let (output, expected) = registry_output_buffer(&mut host, &access, &input, alloc, 17);
    let key = private_registry_create(&mut host, &access, &input, create, private_registry_root(), b"Committed").unwrap();
    let healthy = host.program_private_registry_observation(&access, &input).unwrap();
    let file = fixture.directory.join("output-damaged.hive");
    let mut damage = OpenOptions::new().append(true).open(&file).unwrap();
    damage.write_all(&[0xa5]).unwrap(); damage.sync_all().unwrap(); drop(damage);
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-damaged.hive", &[output]);
    // Invalid output must precede even real store validation: discovering this
    // on-disk damage would poison the otherwise still-healthy retained hive.
    for invalid in [0, output + 15] {
        assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(),
            b"CannotValidate", invalid).is_err());
        assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-damaged.hive", &[output]), before);
    }
    for attempt in 0..2 {
        assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(),
            b"CannotPublish", output + 3).is_err());
        let failed = registry_output_checkpoint(&fixture, &host, &access, &input, "output-damaged.hive", &[output]);
        assert_eq!(failed.store, program_registry_store::Observation { poisoned: true, ..healthy });
        assert_eq!(failed.service, before.service);
        assert_eq!(failed.modules, before.modules);
        assert_eq!(failed.memory, before.memory);
        assert_eq!(failed.journal, before.journal);
        assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
        // Failed open retains its resources only until this error drops; the
        // original service keeps the real damaged file locked on both attempts.
        let error = match program_registry_store::Hive::open(&fixture.directory.canonicalize().unwrap(),
            std::ffi::OsStr::new("output-damaged.hive"), private_registry_limits())
        { Err(error) => error, Ok(_) => panic!("attempt {attempt}: poisoned owner lost its lock") };
        assert_eq!(error.kind(), program_registry_store::ErrorKind::Unavailable);
        drop(error);
        // Malformed bytes also reject an unlocked open. A separate nonblocking
        // flock distinguishes actual retained ownership from parse refusal.
        registry_output_assert_independent_lock(&file, true);
    }
    // Poison does not invent success, release the owner or invalidate an
    // already committed handle's genuine close semantics.
    host.program_reg_close_key(&access, &input, close, key).unwrap();
    assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
    assert_eq!(fs::read(&file).unwrap(), before.journal);
    registry_output_assert_independent_lock(&file, true);
    drop(host);
    registry_output_assert_independent_lock(&file, false);
}

#[test]
fn registry_output_invalid_names_on_healthy_store_preserve_every_preimage() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192);
    let (access, _, create, close) = private_registry_begin(&mut host, &input,
        private_registry_hive(&fixture, "output-healthy-inputs.hive"), 16);
    let (alloc, _) = registry_output_imports(&mut host, &access, &input);
    let (output, _) = registry_output_buffer(&mut host, &access, &input, alloc, 17);
    let before = registry_output_checkpoint(&fixture, &host, &access, &input, "output-healthy-inputs.hive", &[output]);
    assert_eq!((before.store.keys, before.store.records), (1, 0));
    for name in [b"".as_slice(), b"\\absolute", b"a\\\\b", b"a\\", &[0x1f], &[0x7f], &[0xff]] {
        assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(), name, output + 1).is_err());
        assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-healthy-inputs.hive", &[output]), before);
    }
    assert!(registry_output_create(&mut host, &access, &input, create,
        program_registry::PredefinedRoot::LocalMachine.word(), b"Valid", output + 1).is_err());
    assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-healthy-inputs.hive", &[output]), before);
    // These are printable ASCII key names in the actual profile, not filesystem
    // paths to normalize. Their earlier budget-masked error premise was false.
    for (i, name) in [b"bad/path".as_slice(), b".."].into_iter().enumerate() {
        let key = registry_output_create(&mut host, &access, &input, create, private_registry_root(), name, output + 1).unwrap();
        assert_eq!(&host.memory.allocation_bytes(u64::from(output)).unwrap()[1..5], &key.to_le_bytes());
        let checkpoint = registry_output_checkpoint(&fixture, &host, &access, &input, "output-healthy-inputs.hive", &[output]);
        assert_eq!((checkpoint.store.keys, checkpoint.store.records), (i + 2, i + 1));
        assert!(host.program_reg_create_key_a_into(&access, &input, create, key, program_registry::SubKey::Null, output + 1).is_err());
        assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, "output-healthy-inputs.hive", &[output]), checkpoint);
        host.program_reg_close_key(&access, &input, close, key).unwrap();
    }
}

#[test]
fn registry_output_partial_append_and_lost_sync_keep_output_and_exact_lock() {
    for lost_sync in [false, true] {
        let fixture = Fixture::new(); let input = fixture.input();
        let leaf = if lost_sync { "output-sync-loss.hive" } else { "output-partial.hive" };
        let file = fixture.directory.join(leaf);
        let mut hive = private_registry_hive(&fixture, leaf);
        // Existing fixed cfg(test)-only seam performs a real seven-byte append
        // or a full write followed by lost-sync refusal; it cannot return success.
        hive.inject_parent_test_write_failure_once(lost_sync);
        let mut host = fixture.services(8192);
        let (access, _, create, _) = private_registry_begin(&mut host, &input, hive, 16);
        let (alloc, _) = registry_output_imports(&mut host, &access, &input);
        let (output, expected) = registry_output_buffer(&mut host, &access, &input, alloc, 17);
        let before = registry_output_checkpoint(&fixture, &host, &access, &input, leaf, &[output]);
        assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(), b"Pending\\Child", output + 3).is_err());
        let failed = registry_output_checkpoint(&fixture, &host, &access, &input, leaf, &[output]);
        assert_eq!(failed.service, before.service);
        assert_eq!(failed.modules, before.modules);
        assert_eq!(failed.memory, before.memory);
        assert_eq!(failed.store, program_registry_store::Observation { poisoned: true, pending_write: true, ..before.store });
        assert!(failed.journal.starts_with(&before.journal));
        if lost_sync { assert!(failed.journal.len() > before.journal.len() + 7); }
        else { assert_eq!(failed.journal.len(), before.journal.len() + 7); }
        assert_eq!(host.memory.allocation_bytes(u64::from(output)).unwrap(), expected);
        registry_output_assert_independent_lock(&file, true);
        assert!(registry_output_create(&mut host, &access, &input, create, private_registry_root(), b"Retry", output + 3).is_err());
        assert_eq!(registry_output_checkpoint(&fixture, &host, &access, &input, leaf, &[output]), failed);
        registry_output_assert_independent_lock(&file, true);
        drop(host);
        registry_output_assert_independent_lock(&file, false);
        // Uncertain bytes remain; no rollback, durable commit or successful
        // key publication is inferred from the underlying write.
        assert_eq!(fs::read(&file).unwrap(), failed.journal);
    }
}

// Gdi32 private program controls, AUTHORED UNRUN. All source pixels and
// capabilities below come from actual Device/Bitmap/Module/Memory issuances.
fn gdi_binding_source() -> (program_raster::Device, program_raster::Bitmap) {
    let mut device = program_raster::Device::new(program_raster::Limits {
        max_width: 8, max_height: 8, max_bitmap_bytes: 256, max_live_bytes: 4096,
        max_bitmaps: 64, max_selections: 64,
    }).unwrap();
    let bitmap = device.create_bitmap(program_raster::PixelFormat::Rgba8Straight, 2, 1).unwrap();
    let mut selection = device.new_selection().unwrap();
    device.select(&mut selection, &bitmap).unwrap();
    device.fill_rect(&selection, program_raster::Rect { x: 0, y: 0, width: 2, height: 1 },
        program_raster::Pixel::Rgba([23, 47, 89, 173])).unwrap();
    drop(selection);
    (device, bitmap)
}
fn gdi_binding_enable(host: &mut BootstrapServices<'_>, access: &ProgramAccess, input: &Module,
                      device: program_raster::Device) -> (u32, u32) {
    host.program_enable_private_raster(access, input, device, program_raster_context::Policy::PrivateRasterV1, 16).unwrap();
    let module = host.program_load_module(access, input, ModuleKind::Gdi32).unwrap();
    let procedure = host.program_lease_import(access, input, b"gdi32.dll", ProcedureQuery::Name(b"CreateCompatibleDC")).unwrap();
    (module, procedure)
}

#[test]
fn gdi_binding_old_profiles_disabled_and_enable_authenticates_before_device_consumption() {
    let fixture = Fixture::new(); let input = fixture.input(); let reread = fixture.input();
    for api in 0..4 {
        let mut host = fixture.services(8192);
        let access = match api {
            0 => host.begin_program(&input, configuration()).unwrap(),
            1 => host.begin_program_with_ole32(&input, configuration(), program_ole32::Policy::OwnedRustSta).unwrap(),
            2 => host.begin_program_with_profiles(&input, configuration(), program_ole32::Policy::Disabled,
                program_winmm::Policy::Disabled).unwrap(),
            _ => match host.begin_program_with_private_user_registry(&input, configuration(),
                program_ole32::Policy::Disabled, program_winmm::Policy::Disabled,
                private_registry_hive(&fixture, "gdi-disabled.hive"), 16) {
                Ok(access) => access, Err(failure) => panic!("actual hive begin: {:?}", failure.error),
            },
        };
        let before = state(&host); let history = history(&host);
        assert!(host.program_load_module(&access, &input, ModuleKind::Gdi32).is_err());
        assert!(host.program_dependency_kind(&access, &input, b"gdi32").is_err());
        assert!(host.program_lease_import(&access, &input, b"gdi32", ProcedureQuery::Name(b"CreateCompatibleDC")).is_err());
        assert_eq!(state(&host), before); assert_eq!(self::history(&host), history);
    }
    let mut host = fixture.services(8192); let (access, _) = begin(&mut host, &input);
    let mut foreign = fixture.services(8192); let (other_access, _) = begin(&mut foreign, &input);
    for which in 0..6 {
        let (device, bitmap) = gdi_binding_source();
        let observed = device.observe(&bitmap).unwrap(); let before = device.snapshot().unwrap();
        let saved_heap = if which == 5 { Some(std::mem::replace(&mut host.memory, Memory::new(8192).unwrap())) } else { None };
        let failure = match which {
            0 => host.program_enable_private_raster(&other_access, &input, device, program_raster_context::Policy::PrivateRasterV1, 16),
            1 => host.program_enable_private_raster(&access, &reread, device, program_raster_context::Policy::PrivateRasterV1, 16),
            2 => host.program_enable_private_raster(&access, &input, device, program_raster_context::Policy::Disabled, 16),
            3 => host.program_enable_private_raster(&access, &input, device, program_raster_context::Policy::PrivateRasterV1, 0),
            4 => host.program_enable_private_raster(&access, &input, device, program_raster_context::Policy::PrivateRasterV1, 4097),
            _ => host.program_enable_private_raster(&access, &input, device, program_raster_context::Policy::PrivateRasterV1, 16),
        }.unwrap_err();
        if let Some(memory) = saved_heap { host.memory = memory; }
        assert!(failure.device.observe(&bitmap).unwrap().same_bitmap(&observed));
        assert_eq!(failure.device.snapshot().unwrap(), before);
    }
    let (device, bitmap) = gdi_binding_source(); gdi_binding_enable(&mut host, &access, &input, device);
    let enabled = host.program_private_raster_observation(&access, &input).unwrap();
    let (device, other) = gdi_binding_source(); let observed = device.observe(&other).unwrap();
    let failure = host.program_enable_private_raster(&access, &input, device,
        program_raster_context::Policy::PrivateRasterV1, 16).unwrap_err();
    assert!(failure.device.observe(&other).unwrap().same_bitmap(&observed));
    assert_eq!(host.program_private_raster_observation(&access, &input).unwrap(), enabled);
    host.program_raster_attach_source(&access, &input, &bitmap).unwrap();
}

#[test]
fn gdi_binding_real_source_derived_pixels_and_close_revoke_exact_use_once() {
    let fixture = Fixture::new(); let input = fixture.input();
    let mut host = fixture.services(8192); let (access, _) = begin(&mut host, &input);
    let (device, bitmap) = gdi_binding_source(); let (_, procedure) = gdi_binding_enable(&mut host, &access, &input, device);
    let source = host.program_raster_attach_source(&access, &input, &bitmap).unwrap();
    let derived = host.program_create_compatible_dc(&access, &input, procedure, source).unwrap();
    let pixels = host.program_raster_readback_rgba(&access, &input, source).unwrap();
    assert_eq!((pixels.width, pixels.height, pixels.pixels), (2, 1, [23, 47, 89, 173].repeat(2)));
    let pixels = host.program_raster_readback_rgba(&access, &input, derived).unwrap();
    assert_eq!((pixels.width, pixels.height, pixels.pixels), (1, 1, vec![0, 0, 0, 255]));
    assert_ne!(source, derived);
    let issued = host.next;
    host.program_raster_close(&access, &input, source).unwrap();
    assert_eq!(host.next, issued);
    assert!(host.program_raster_close(&access, &input, source).is_err());
    assert!(host.program_raster_readback_rgba(&access, &input, source).is_err());
    assert!(host.program_create_compatible_dc(&access, &input, procedure, source).is_err());
    // The same real caller Bitmap survives close, and selected backing survives
    // its later caller-owner drop until the final context actually closes.
    let source = host.program_raster_attach_source(&access, &input, &bitmap).unwrap();
    drop(bitmap);
    assert_eq!(host.program_raster_readback_rgba(&access, &input, source).unwrap().pixels, [23, 47, 89, 173].repeat(2));
    host.program_raster_close(&access, &input, source).unwrap();
    assert_eq!(host.program_raster_readback_rgba(&access, &input, derived).unwrap().pixels, [0, 0, 0, 255]);
    host.program_raster_close(&access, &input, derived).unwrap();
    let observed = host.program_private_raster_observation(&access, &input).unwrap();
    assert_eq!((observed.contexts_issued, observed.live_contexts, observed.device.live_bitmap_bytes), (3, 0, 0));
}

#[test]
fn gdi_binding_authentic_foreign_owners_none_heap_and_procedure_kinds_refuse() {
    let fixture = Fixture::new(); let input = fixture.input(); let reread = fixture.input();
    let mut host = fixture.services(8192); let (access, kernel) = begin(&mut host, &input);
    let mut foreign = fixture.services(8192); let (other_access, _) = begin(&mut foreign, &input);
    let (device, bitmap) = gdi_binding_source(); let (gdi, procedure) = gdi_binding_enable(&mut host, &access, &input, device);
    let (device, other_bitmap) = gdi_binding_source(); let (_, other_procedure) = gdi_binding_enable(&mut foreign, &other_access, &input, device);
    let source = host.program_raster_attach_source(&access, &input, &bitmap).unwrap();
    let other_source = foreign.program_raster_attach_source(&other_access, &input, &other_bitmap).unwrap();
    assert_eq!((source, procedure), (other_source, other_procedure));
    let wrong = lease(&mut host, &access, &input, Export::GlobalFree);
    let before = state(&host); let observed = host.program_private_raster_observation(&access, &input).unwrap();
    assert!(host.program_create_compatible_dc(&access, &input, procedure, 0).is_err());
    for word in [kernel, gdi, wrong, source, 0] {
        assert!(host.program_create_compatible_dc(&access, &input, word, source).is_err());
    }
    for word in [kernel, gdi, wrong, procedure, 0] {
        assert!(host.program_create_compatible_dc(&access, &input, procedure, word).is_err());
        assert!(host.program_raster_readback_rgba(&access, &input, word).is_err());
        assert!(host.program_raster_close(&access, &input, word).is_err());
    }
    assert!(host.program_raster_attach_source(&access, &input, &other_bitmap).is_err());
    assert!(host.program_create_compatible_dc(&other_access, &input, procedure, source).is_err());
    assert!(host.program_create_compatible_dc(&access, &reread, procedure, source).is_err());
    assert!(host.program_raster_readback_rgba(&other_access, &input, source).is_err());
    assert!(host.program_raster_close(&access, &reread, source).is_err());
    // Same-value foreign words alone cannot carry origin. Swap the genuine
    // owners at those coincident words to exercise actual issuer rejection.
    let local_context = host.capabilities.remove(&source).unwrap();
    let foreign_context = foreign.capabilities.remove(&other_source).unwrap();
    host.capabilities.insert(source, foreign_context);
    assert!(host.program_create_compatible_dc(&access, &input, procedure, source).is_err());
    assert!(host.program_raster_readback_rgba(&access, &input, source).is_err());
    assert!(host.program_raster_close(&access, &input, source).is_err());
    let foreign_context = host.capabilities.remove(&source).unwrap();
    host.capabilities.insert(source, local_context); foreign.capabilities.insert(other_source, foreign_context);
    let local_procedure = host.capabilities.remove(&procedure).unwrap();
    let foreign_procedure = foreign.capabilities.remove(&other_procedure).unwrap();
    host.capabilities.insert(procedure, foreign_procedure);
    assert!(host.program_create_compatible_dc(&access, &input, procedure, source).is_err());
    let foreign_procedure = host.capabilities.remove(&procedure).unwrap();
    host.capabilities.insert(procedure, local_procedure); foreign.capabilities.insert(other_procedure, foreign_procedure);
    let memory = std::mem::replace(&mut host.memory, Memory::new(8192).unwrap());
    assert!(host.program_raster_attach_source(&access, &input, &bitmap).is_err());
    assert!(host.program_create_compatible_dc(&access, &input, procedure, source).is_err());
    assert!(host.program_raster_readback_rgba(&access, &input, source).is_err());
    assert!(host.program_raster_close(&access, &input, source).is_err());
    host.memory = memory;
    assert_eq!(state(&host), before);
    assert_eq!(host.program_private_raster_observation(&access, &input).unwrap(), observed);
    assert_eq!(host.program_raster_readback_rgba(&access, &input, source).unwrap().pixels, [23, 47, 89, 173].repeat(2));
}

#[test]
fn gdi_binding_actual_shared_budget_exhaustion_precedes_all_context_effects() {
    let fixture = Fixture::new(); let input = fixture.input(); let mut host = fixture.services(8192);
    let mut config = configuration(); config.limits.procedure_lease_issuances = 4096;
    let access = host.begin_program(&input, config).unwrap();
    let (device, bitmap) = gdi_binding_source(); let (_, procedure) = gdi_binding_enable(&mut host, &access, &input, device);
    let source = host.program_raster_attach_source(&access, &input, &bitmap).unwrap();
    // Exhaust the shared allocator using real owning import issuances, rather
    // than constructing authority from a counter or made-up capability word.
    while host.live_capabilities() < MAX_CAPABILITIES as usize {
        host.program_lease_import(&access, &input, b"gdi32", ProcedureQuery::Name(b"CreateCompatibleDC")).unwrap();
    }
    let before = state(&host); let observed = host.program_private_raster_observation(&access, &input).unwrap();
    assert!(host.program_raster_attach_source(&access, &input, &bitmap).is_err());
    assert!(host.program_create_compatible_dc(&access, &input, procedure, source).is_err());
    assert_eq!(state(&host), before);
    assert_eq!(host.program_private_raster_observation(&access, &input).unwrap(), observed);
    host.program_raster_close(&access, &input, source).unwrap();
    assert!(host.program_raster_attach_source(&access, &input, &bitmap).is_err());
    assert_eq!(host.program_private_raster_observation(&access, &input).unwrap().contexts_issued, 1);
}

#[test]
fn gdi_binding_retirement_reload_and_terminal_leave_old_receipts_unusable() {
    let fixture = Fixture::new(); let input = fixture.input(); let mut host = fixture.services(8192);
    let (access, _) = begin(&mut host, &input);
    let get_proc = lease(&mut host, &access, &input, Export::GetProcAddress);
    let exit = lease(&mut host, &access, &input, Export::ExitProcess);
    let (device, bitmap) = gdi_binding_source(); let (module, procedure) = gdi_binding_enable(&mut host, &access, &input, device);
    let old_identity = host.program_module_identity(&access, &input, module).unwrap();
    let weak = host.program_get_proc_address(&access, &input, get_proc, module,
        ProcedureQuery::Name(b"CreateCompatibleDC")).unwrap();
    let source = host.program_raster_attach_source(&access, &input, &bitmap).unwrap();
    host.capabilities.remove(&procedure);
    assert!(host.program.as_mut().unwrap().registry.retire_module(&host.memory, ModuleKind::Gdi32).is_err());
    host.program_raster_close(&access, &input, source).unwrap();
    host.program.as_mut().unwrap().registry.retire_module(&host.memory, ModuleKind::Gdi32).unwrap();
    let module = host.program_load_module(&access, &input, ModuleKind::Gdi32).unwrap();
    assert_ne!(host.program_module_identity(&access, &input, module).unwrap(), old_identity);
    let source = host.program_raster_attach_source(&access, &input, &bitmap).unwrap();
    assert!(host.program_create_compatible_dc(&access, &input, weak, source).is_err());
    let fresh = host.program_lease_import(&access, &input, b"gdi32", ProcedureQuery::Name(b"CreateCompatibleDC")).unwrap();
    let derived = host.program_create_compatible_dc(&access, &input, fresh, source).unwrap();
    // Keep an actual context receipt outside the service for this teardown
    // control. Its weak registry link must not keep resources operational.
    let retained = host.capabilities.remove(&derived).unwrap();
    assert_eq!(host.program_exit_process(&access, &input, exit, 97).unwrap().exit_code, 97);
    assert!(host.capabilities.is_empty());
    assert!(host.program_raster_readback_rgba(&access, &input, source).is_err());
    assert!(host.program_create_compatible_dc(&access, &input, fresh, source).is_err());
    let (device, other) = gdi_binding_source(); let observed = device.observe(&other).unwrap();
    let failure = host.program_enable_private_raster(&access, &input, device,
        program_raster_context::Policy::PrivateRasterV1, 16).unwrap_err();
    assert!(failure.device.observe(&other).unwrap().same_bitmap(&observed));
    drop(host);
    let mut replacement = fixture.services(8192); let (new_access, _) = begin(&mut replacement, &input);
    let (device, bitmap) = gdi_binding_source(); gdi_binding_enable(&mut replacement, &new_access, &input, device);
    let word = replacement.program_raster_attach_source(&new_access, &input, &bitmap).unwrap();
    let current = replacement.capabilities.insert(word, retained).unwrap();
    assert!(replacement.program_raster_readback_rgba(&new_access, &input, word).is_err());
    replacement.capabilities.insert(word, current);
    assert!(replacement.program_raster_readback_rgba(&new_access, &input, word).is_ok());
}
