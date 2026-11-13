//! Authored controls only. Parent must add the shared Memory hook/module
//! declaration and compile/run these; this worker has run none of them.
use super::*;
use sims_sim::native_world_children::Owner;

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
fn heap() -> Memory {
    Memory::new(8192).unwrap()
}
fn registry(memory: &Memory) -> Registry {
    let mut registry = Registry::new(memory, configuration()).unwrap();
    registry.load_kernel32(memory).unwrap();
    registry
}
fn lease(registry: &mut Registry, memory: &Memory, export: Export) -> ProcedureLease {
    registry.lease_import(memory, b"kernel32.dll", ProcedureQuery::Name(export.name())).unwrap()
}
fn allocate(registry: &mut Registry, memory: &mut Memory, procedure: &ProcedureLease, flags: u32, bytes: u32) -> GlobalAllocation {
    match registry.global_alloc(memory, ProcedureAccess::Import(procedure), flags, bytes).unwrap() {
        GlobalAllocReturn::Allocated(owner) => owner,
        GlobalAllocReturn::OutOfMemory => panic!("small supported allocation unexpectedly exhausted host memory"),
    }
}
fn observe(registry: &mut Registry, memory: &Memory, export: Export) -> ProcedureObservation {
    let get_module = lease(registry, memory, Export::GetModuleHandleA);
    let get_proc = lease(registry, memory, Export::GetProcAddress);
    let module = registry.get_module_handle_a(memory, ProcedureAccess::Import(&get_module), Some(b"kernel32")).unwrap().unwrap();
    registry.get_proc_address(memory, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Name(export.name())).unwrap().unwrap()
}
fn changed_owner(memory: &mut Memory, byte: u8) -> Owner {
    let mut owner = memory.allocate_owner(24).unwrap();
    owner.data.fill(byte);
    memory.synchronize_owner(&owner).unwrap();
    owner
}

// WINMM wiring controls, ALL UNRUN. Enabled profiles below only construct,
// resolve, observe, or REFUSE calls before the producer. They never use a
// synthetic clock seam or perform a valid real-clock dispatch.
fn winmm_registry(memory: &Memory) -> Registry {
    Registry::new_with_profiles(memory, configuration(), program_ole32::Policy::OwnedRustSta,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap()
}
fn winmm_lease(registry: &mut Registry, memory: &Memory) -> ProcedureLease {
    registry.lease_import(memory, b"WINMM.dll", ProcedureQuery::Name(b"timeGetTime")).unwrap()
}
fn winmm_unsampled(registry: &Registry, memory: &Memory) {
    let evidence = registry.time_get_time_observation(memory).unwrap();
    assert!(evidence.last_sample().is_none());
    assert!(evidence.latest_report().is_none());
    assert!(evidence.first_failure_report().is_none());
}

#[test]
fn winmm_old_constructors_disable_without_issuance_or_sampling() {
    let memory = heap();
    for mut registry in [Registry::new(&memory, configuration()).unwrap(),
        Registry::new_with_ole32(&memory, configuration(), program_ole32::Policy::OwnedRustSta).unwrap(),
        Registry::new_with_profiles(&memory, configuration(), program_ole32::Policy::OwnedRustSta,
            program_winmm::Policy::Disabled).unwrap()]
    {
        let before = registry.history(&memory).unwrap();
        assert!(registry.load_winmm(&memory).is_err());
        assert!(registry.dependency_kind(&memory, b"WINMM.dll").is_err());
        assert!(registry.lease_import(&memory, b"winmm", ProcedureQuery::Name(b"timeGetTime")).is_err());
        assert_eq!(registry.history(&memory).unwrap(), before);
        assert_eq!(registry.time_get_time_observation(&memory).unwrap().policy(), program_winmm::Policy::Disabled);
        winmm_unsampled(&registry, &memory);
    }
}

#[test]
fn winmm_exact_single_export_names_ordinals_and_three_distinct_identities() {
    let memory = heap();
    let mut registry = winmm_registry(&memory);
    let kernel = registry.load_kernel32(&memory).unwrap();
    let ole = registry.load_ole32(&memory).unwrap();
    let winmm = registry.load_winmm(&memory).unwrap();
    let k = registry.module_identity(&memory, &kernel).unwrap();
    let o = registry.module_identity(&memory, &ole).unwrap();
    let w = registry.module_identity(&memory, &winmm).unwrap();
    assert_eq!((k.kind(), o.kind(), w.kind()), (ModuleKind::Kernel32, ModuleKind::Ole32, ModuleKind::Winmm));
    assert_ne!(k, o); assert_ne!(k, w); assert_ne!(o, w);
    assert_eq!(registry.kernel32.as_ref().unwrap().procedures.len(), 5);
    assert_eq!(registry.ole32.as_ref().unwrap().procedures.len(), 1);
    assert_eq!(registry.winmm.as_ref().unwrap().procedures.len(), 1);
    assert_eq!(registry.winmm.as_ref().unwrap().procedures[0].export.name(), b"timeGetTime");
    for name in [b"winmm".as_slice(), b"WINMM.DLL", b"WiNmM.DlL"] {
        assert_eq!(registry.dependency_kind(&memory, name).unwrap(), ModuleKind::Winmm);
    }
    let before = registry.history(&memory).unwrap();
    for name in [b"".as_slice(), b"dir/winmm.dll", b"C:\\winmm.dll", b"winmm\0", b"..", b"winmm.exe"] {
        assert!(registry.dependency_kind(&memory, name).is_err());
    }
    for (library, query) in [
        (b"winmm".as_slice(), ProcedureQuery::Name(b"TimeGetTime")),
        (b"winmm", ProcedureQuery::Name(b"timegettime")),
        (b"winmm", ProcedureQuery::Name(b"timeGetTime\0")),
        (b"winmm", ProcedureQuery::Name(b"GlobalAlloc")),
        (b"winmm", ProcedureQuery::Name(b"CoUninitialize")),
        (b"kernel32", ProcedureQuery::Name(b"timeGetTime")),
        (b"ole32", ProcedureQuery::Name(b"timeGetTime")),
        (b"winmm", ProcedureQuery::Ordinal(0)),
        (b"winmm", ProcedureQuery::Ordinal(1)),
    ] { assert!(registry.lease_import(&memory, library, query).is_err()); }
    assert_eq!(registry.history(&memory).unwrap(), before);
    let imported = winmm_lease(&mut registry, &memory);
    assert_eq!(registry.procedure_export(&memory, &imported).unwrap(), Export::TimeGetTime);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    for module in [&kernel, &ole] {
        assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), module,
            ProcedureQuery::Name(b"timeGetTime")).unwrap().is_none());
    }
    for name in [b"TimeGetTime".as_slice(), b"GlobalFree", b"CoUninitialize"] {
        assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &winmm,
            ProcedureQuery::Name(name)).unwrap().is_none());
    }
    assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &winmm,
        ProcedureQuery::Ordinal(1)).is_err());
    winmm_unsampled(&registry, &memory);
}

#[test]
fn winmm_weak_ownership_reload_keeps_program_clock_and_never_upgrades() {
    let memory = heap();
    let mut registry = winmm_registry(&memory);
    registry.load_kernel32(&memory).unwrap();
    let old = registry.load_winmm(&memory).unwrap();
    let old_id = registry.module_identity(&memory, &old).unwrap();
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let alias = registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"winmm")).unwrap().unwrap();
    assert_eq!(registry.module_identity(&memory, &alias).unwrap(), old_id);
    let weak = registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &old,
        ProcedureQuery::Name(b"timeGetTime")).unwrap().unwrap();
    let leases = registry.history(&memory).unwrap().procedure_leases_issued;
    assert_eq!(registry.observed_export(&memory, &weak).unwrap(), Export::TimeGetTime);
    assert_eq!(registry.history(&memory).unwrap().procedure_leases_issued, leases);
    assert_eq!(Arc::strong_count(registry.winmm.as_ref().unwrap()), 1);
    let imported = winmm_lease(&mut registry, &memory);
    assert_eq!(Arc::strong_count(registry.winmm.as_ref().unwrap()), 2);
    assert!(registry.retire_winmm(&memory).is_err());
    let clock = &registry.winmm_clock as *const program_winmm::Clock;
    drop(imported);
    registry.retire_winmm(&memory).unwrap();
    assert!(registry.time_get_time(&memory, ProcedureAccess::Lookup(&weak)).is_err());
    let fresh = registry.load_winmm(&memory).unwrap();
    assert_ne!(registry.module_identity(&memory, &fresh).unwrap(), old_id);
    assert!(registry.require_module(&memory, &alias).is_err());
    assert!(registry.time_get_time(&memory, ProcedureAccess::Lookup(&weak)).is_err());
    assert_eq!(clock, &registry.winmm_clock as *const program_winmm::Clock);
    assert_eq!(registry.winmm_clock.policy(), program_winmm::Policy::DarwinKernelBootElapsed);
    assert!(registry.observed_export(&memory, &weak).is_err());
    registry.retire_winmm(&memory).unwrap();
    winmm_unsampled(&registry, &memory);
}

#[test]
fn winmm_foreign_heap_registry_wrong_table_and_export_refuse_before_sample() {
    let memory = heap();
    let foreign_heap = heap();
    let mut registry = winmm_registry(&memory);
    let mut foreign = winmm_registry(&memory);
    registry.load_kernel32(&memory).unwrap();
    let module = registry.load_winmm(&memory).unwrap();
    let foreign_module = foreign.load_winmm(&memory).unwrap();
    let local = winmm_lease(&mut registry, &memory);
    let other = winmm_lease(&mut foreign, &memory);
    let wrong_export = lease(&mut registry, &memory, Export::GlobalFree);
    let before = registry.history(&memory).unwrap();
    assert_ne!(registry.module_identity(&memory, &module).unwrap(), foreign.module_identity(&memory, &foreign_module).unwrap());
    assert!(registry.time_get_time(&foreign_heap, ProcedureAccess::Import(&local)).is_err());
    assert!(registry.time_get_time_observation(&foreign_heap).is_err());
    assert!(registry.time_get_time(&memory, ProcedureAccess::Import(&other)).is_err());
    assert!(registry.time_get_time(&memory, ProcedureAccess::Import(&wrong_export)).is_err());
    // Deliberately corrupted test-only combinations must not qualify an entry
    // merely because its export name matches; no clock/sample is fabricated.
    let mixed = ProcedureLease { registry: Arc::downgrade(&registry.lifetime),
        module: Arc::clone(registry.winmm.as_ref().unwrap()), procedure: Arc::clone(&other.procedure) };
    let wrong_table = ProcedureLease { registry: Arc::downgrade(&registry.lifetime),
        module: Arc::clone(registry.kernel32.as_ref().unwrap()), procedure: Arc::clone(&local.procedure) };
    let wrong_weak = ProcedureObservation { registry: Arc::downgrade(&registry.lifetime),
        module: Arc::downgrade(registry.kernel32.as_ref().unwrap()), procedure: Arc::downgrade(&local.procedure) };
    assert!(registry.time_get_time(&memory, ProcedureAccess::Import(&mixed)).is_err());
    assert!(registry.time_get_time(&memory, ProcedureAccess::Import(&wrong_table)).is_err());
    assert!(registry.time_get_time(&memory, ProcedureAccess::Lookup(&wrong_weak)).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
    winmm_unsampled(&registry, &memory); winmm_unsampled(&foreign, &memory);
}

#[test]
fn winmm_shared_module_lease_and_observation_budgets_do_not_sample() {
    let memory = heap();
    let mut config = configuration();
    config.kernel32 = Kernel32Policy::Disabled;
    config.limits.module_issuances = 1;
    config.limits.procedure_lease_issuances = 1;
    config.limits.weak_observation_issuances = 1;
    let mut registry = Registry::new_with_profiles(&memory, config, program_ole32::Policy::Disabled,
        program_winmm::Policy::DarwinKernelBootElapsed).unwrap();
    registry.load_winmm(&memory).unwrap();
    assert!(registry.load_kernel32(&memory).is_err());
    let imported = winmm_lease(&mut registry, &memory);
    let before = registry.history(&memory).unwrap();
    assert!(registry.lease_import(&memory, b"winmm", ProcedureQuery::Name(b"timeGetTime")).is_err());
    assert!(registry.load_winmm(&memory).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
    drop(imported);
    registry.retire_winmm(&memory).unwrap();
    assert!(registry.load_winmm(&memory).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
    winmm_unsampled(&registry, &memory);
}

#[test]
fn winmm_terminal_and_drop_revoke_survivors_without_touching_clock_evidence() {
    let mut memory = heap();
    let mut registry = winmm_registry(&memory);
    registry.load_kernel32(&memory).unwrap();
    let module = registry.load_winmm(&memory).unwrap();
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let weak = registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module,
        ProcedureQuery::Name(b"timeGetTime")).unwrap().unwrap();
    let imported = winmm_lease(&mut registry, &memory);
    let exit = lease(&mut registry, &memory, Export::ExitProcess);
    let transfer = registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 17).unwrap();
    assert!(registry.time_get_time(&memory, ProcedureAccess::Import(&imported)).is_err());
    assert!(registry.time_get_time(&memory, ProcedureAccess::Lookup(&weak)).is_err());
    assert!(!imported.module.active.load(Ordering::Acquire));
    assert_eq!(registry.finish(&memory, transfer).unwrap().exit_code, 17);
    winmm_unsampled(&registry, &memory);
    let mut later = winmm_registry(&memory);
    later.load_winmm(&memory).unwrap();
    let survivor = winmm_lease(&mut later, &memory);
    drop(later);
    assert!(!survivor.module.active.load(Ordering::Acquire));
    assert!(registry.time_get_time(&memory, ProcedureAccess::Import(&survivor)).is_err());
    winmm_unsampled(&registry, &memory);
}

#[test]
fn winmm_producer_refusals_are_explicit_service_errors_never_zero() {
    use program_winmm::{Field, Stage, Stop as ClockStop};
    for stop in [ClockStop::Disabled, ClockStop::UnsupportedHost, ClockStop::ForeignSample] {
        assert!(matches!(winmm_stop(stop), Stop::Unsupported(_)));
        assert!(format!("{:?}", winmm_stop(stop)).contains(&format!("{stop:?}")));
    }
    for stop in [ClockStop::SequenceExhausted,
        ClockStop::NativeCallFailed { stage: Stage::BootBefore, return_code: -1, errno: 5 },
        ClockStop::UnexpectedLength { stage: Stage::BootAfter, expected: 16, returned: Some(8) },
        ClockStop::InvalidScalar { stage: Stage::Calendar, field: Field::Microseconds, value: -1 },
        ClockStop::BootChanged, ClockStop::CalendarBeforeBoot,
        ClockStop::ElapsedRegression { previous_microseconds: 20, observed_microseconds: 10 }]
    {
        assert!(matches!(winmm_stop(stop), Stop::Unavailable(_)));
        assert!(format!("{:?}", winmm_stop(stop)).contains(&format!("{stop:?}")));
    }
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
#[test]
#[ignore = "parent-only explicit real clock issuance/owner retention control; authored UNRUN"]
fn winmm_parent_real_sample_owner_survives_reload_and_foreign_clock_cannot_adopt() {
    let memory = heap();
    let mut registry = winmm_registry(&memory);
    let old_module = registry.load_winmm(&memory).unwrap();
    let old_identity = registry.module_identity(&memory, &old_module).unwrap();
    let imported = winmm_lease(&mut registry, &memory);
    let result = registry.time_get_time(&memory, ProcedureAccess::Import(&imported));
    let evidence = registry.time_get_time_observation(&memory).unwrap();
    let value = result.unwrap_or_else(|stop| panic!("parent clock refusal {stop:?}; latest={:?}; first={:?}",
        evidence.latest_report(), evidence.first_failure_report()));
    let sample = evidence.last_sample().unwrap().clone();
    let first_failure = evidence.first_failure_report().copied();
    assert_eq!(value, sample.value());
    assert_eq!(registry.winmm_clock.validate(&sample), Ok(()));
    drop(imported);
    registry.retire_winmm(&memory).unwrap();
    let fresh = registry.load_winmm(&memory).unwrap();
    assert_ne!(registry.module_identity(&memory, &fresh).unwrap(), old_identity);
    assert_eq!(registry.winmm_clock.validate(&sample), Ok(()));
    let evidence = registry.time_get_time_observation(&memory).unwrap();
    assert_eq!(evidence.last_sample().unwrap().sequence(), sample.sequence());
    assert_eq!(evidence.latest_report(), Some(sample.report()));
    assert_eq!(evidence.first_failure_report().copied(), first_failure);
    let foreign = winmm_registry(&memory);
    assert_eq!(foreign.winmm_clock.validate(&sample), Err(program_winmm::Stop::ForeignSample));
    winmm_unsampled(&foreign, &memory);
    // Validation did not adopt or resample either clock's historical issuance.
    assert_eq!(registry.winmm_clock.latest_report(), Some(sample.report()));
}

#[cfg(not(all(target_os = "macos", target_pointer_width = "64")))]
#[test]
fn winmm_unsupported_host_retains_bounded_actual_typed_reports_without_io() {
    let memory = heap();
    let mut registry = winmm_registry(&memory);
    registry.load_winmm(&memory).unwrap();
    let imported = winmm_lease(&mut registry, &memory);
    let before = registry.history(&memory).unwrap();
    for _ in 0..32 {
        assert!(matches!(registry.time_get_time(&memory, ProcedureAccess::Import(&imported)), Err(Stop::Unsupported(_))));
        let observation = registry.time_get_time_observation(&memory).unwrap();
        let report = observation.latest_report().unwrap();
        assert_eq!(report.result(), Err(program_winmm::Stop::UnsupportedHost));
        assert_eq!(report.sequence(), None);
        assert_eq!(report.provenance(), program_winmm::Provenance::NoNativeIo);
        assert_eq!(observation.first_failure_report(), Some(report));
        assert!(observation.last_sample().is_none());
        for stage in [program_winmm::Stage::BootBefore, program_winmm::Stage::Calendar, program_winmm::Stage::BootAfter] {
            assert_eq!(report.call(stage), program_winmm::NativeOutcome::NotEntered);
        }
    }
    assert_eq!(registry.history(&memory).unwrap(), before);
}

// OLE32 wiring controls, authored UNRUN. These inspect the real sealed tables
// and genuine leases; negative substitutions below deliberately cross actual
// issued identities, never an adopted native pointer or callback.
fn ole32_registry(memory: &Memory) -> Registry {
    Registry::new_with_ole32(memory, configuration(), program_ole32::Policy::OwnedRustSta).unwrap()
}

#[test]
fn ole32_opt_in_has_one_distinct_export_and_old_configuration_stays_disabled() {
    let memory = heap();
    let mut old = Registry::new(&memory, configuration()).unwrap();
    let before = old.history(&memory).unwrap();
    assert!(old.load_ole32(&memory).is_err());
    assert!(old.dependency_kind(&memory, b"ole32.dll").is_err());
    assert!(old.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).is_err());
    assert_eq!(old.history(&memory).unwrap(), before);
    let mut registry = ole32_registry(&memory);
    let kernel = registry.load_kernel32(&memory).unwrap();
    let ole = registry.load_ole32(&memory).unwrap();
    assert_eq!(registry.kernel32.as_ref().unwrap().procedures.len(), 5);
    assert_eq!(registry.ole32.as_ref().unwrap().procedures.len(), 1);
    assert_eq!(registry.ole32.as_ref().unwrap().procedures[0].export, Export::CoUninitialize);
    assert!(!Arc::ptr_eq(registry.kernel32.as_ref().unwrap(), registry.ole32.as_ref().unwrap()));
    let k_id = registry.module_identity(&memory, &kernel).unwrap();
    let o_id = registry.module_identity(&memory, &ole).unwrap();
    assert_eq!((k_id.kind(), o_id.kind()), (ModuleKind::Kernel32, ModuleKind::Ole32));
    assert_ne!(k_id, o_id);
    assert_eq!(o_id, registry.module_identity(&memory, &ole).unwrap());
    let before = registry.history(&memory).unwrap();
    for name in [b"ole32".as_slice(), b"OLE32", b"OlE32.DlL"] {
        assert_eq!(registry.dependency_kind(&memory, name).unwrap(), ModuleKind::Ole32);
    }
    for name in [b"".as_slice(), b"dir/ole32.dll", b"C:\\ole32.dll", b"ole32\0", b"..", b"advapi32.dll"] {
        assert!(registry.dependency_kind(&memory, name).is_err());
    }
    for (library, query) in [
        (b"ole32".as_slice(), ProcedureQuery::Name(b"GlobalAlloc")),
        (b"kernel32", ProcedureQuery::Name(b"CoUninitialize")),
        (b"ole32", ProcedureQuery::Name(b"couninitialize")),
        (b"ole32", ProcedureQuery::Name(b"CoInitialize")),
        (b"ole32", ProcedureQuery::Ordinal(1)),
        (b"ole32", ProcedureQuery::Ordinal(0)),
    ] {
        assert!(registry.lease_import(&memory, library, query).is_err());
    }
    assert_eq!(registry.history(&memory).unwrap(), before);
    let import = registry.lease_import(&memory, b"ole32.dll", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    assert_eq!(registry.procedure_export(&memory, &import).unwrap(), Export::CoUninitialize);
    assert_eq!(registry.co_uninitialize_policy(&memory, ProcedureAccess::Import(&import)).unwrap(), program_ole32::Policy::OwnedRustSta);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &kernel,
        ProcedureQuery::Name(b"CoUninitialize")).unwrap().is_none());
    assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &ole,
        ProcedureQuery::Name(b"GlobalAlloc")).unwrap().is_none());
    assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &ole,
        ProcedureQuery::Ordinal(1)).is_err());
    let weak = registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &ole,
        ProcedureQuery::Name(b"CoUninitialize")).unwrap().unwrap();
    assert_eq!(registry.observed_export(&memory, &weak).unwrap(), Export::CoUninitialize);
}

#[test]
fn ole32_only_policy_and_shared_histories_do_not_enable_kernel32() {
    let memory = heap();
    let mut config = configuration();
    config.kernel32 = Kernel32Policy::Disabled;
    config.limits.module_issuances = 1;
    config.limits.procedure_lease_issuances = 1;
    let mut registry = Registry::new_with_ole32(&memory, config, program_ole32::Policy::OwnedRustSta).unwrap();
    assert!(registry.load_kernel32(&memory).is_err());
    let module = registry.load_ole32(&memory).unwrap();
    let imported = registry.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    let before = registry.history(&memory).unwrap();
    assert!(registry.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).is_err());
    assert!(registry.retire_ole32(&memory).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
    drop(imported);
    registry.retire_ole32(&memory).unwrap();
    assert!(registry.require_module(&memory, &module).is_err());
    assert!(registry.load_ole32(&memory).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
}

#[test]
fn ole32_owning_import_pins_only_its_module_and_weak_history_never_revives() {
    let memory = heap();
    let mut registry = ole32_registry(&memory);
    registry.load_kernel32(&memory).unwrap();
    let old = registry.load_ole32(&memory).unwrap();
    let old_id = registry.module_identity(&memory, &old).unwrap();
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let alias = registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"OLE32.DLL")).unwrap().unwrap();
    assert_eq!(registry.module_identity(&memory, &alias).unwrap(), old_id);
    let weak = registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &old,
        ProcedureQuery::Name(b"CoUninitialize")).unwrap().unwrap();
    let imported = registry.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    assert!(registry.retire_ole32(&memory).is_err());
    assert_eq!(Arc::strong_count(registry.ole32.as_ref().unwrap()), 2);
    drop(imported);
    assert_eq!(Arc::strong_count(registry.ole32.as_ref().unwrap()), 1);
    registry.retire_ole32(&memory).unwrap();
    assert!(registry.require_module(&memory, &old).is_err());
    assert!(registry.co_uninitialize_policy(&memory, ProcedureAccess::Lookup(&weak)).is_err());
    assert!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"ole32")).unwrap().is_none());
    let new = registry.load_ole32(&memory).unwrap();
    assert_ne!(registry.module_identity(&memory, &new).unwrap(), old_id);
    assert!(registry.require_module(&memory, &alias).is_err());
    assert!(registry.observed_export(&memory, &weak).is_err());
    registry.procedure_export(&memory, &get_proc).unwrap();
    registry.retire_ole32(&memory).unwrap();
}

#[test]
fn ole32_foreign_heap_registry_and_same_export_wrong_module_refuse() {
    let memory = heap();
    let foreign_heap = heap();
    let mut registry = ole32_registry(&memory);
    let mut other = ole32_registry(&memory);
    registry.load_kernel32(&memory).unwrap();
    let module = registry.load_ole32(&memory).unwrap();
    let foreign_module = other.load_ole32(&memory).unwrap();
    let local = registry.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    let foreign = other.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    let before = registry.history(&memory).unwrap();
    assert!(registry.procedure_export(&foreign_heap, &local).is_err());
    assert!(registry.load_ole32(&foreign_heap).is_err());
    assert!(registry.retire_ole32(&foreign_heap).is_err());
    assert!(registry.require_module(&foreign_heap, &module).is_err());
    assert!(registry.require_module(&memory, &foreign_module).is_err());
    assert_ne!(registry.module_identity(&memory, &module).unwrap(), other.module_identity(&memory, &foreign_module).unwrap());
    assert!(registry.procedure_export(&memory, &foreign).is_err());
    let mixed = ProcedureLease { registry: Arc::downgrade(&registry.lifetime),
        module: Arc::clone(registry.ole32.as_ref().unwrap()), procedure: Arc::clone(&foreign.procedure) };
    assert!(registry.procedure_export(&memory, &mixed).is_err());
    let wrong_table = ProcedureLease { registry: Arc::downgrade(&registry.lifetime),
        module: Arc::clone(registry.kernel32.as_ref().unwrap()), procedure: Arc::clone(&local.procedure) };
    assert!(registry.procedure_export(&memory, &wrong_table).is_err());
    let wrong_weak = ProcedureObservation { registry: Arc::downgrade(&registry.lifetime),
        module: Arc::downgrade(registry.kernel32.as_ref().unwrap()), procedure: Arc::downgrade(&local.procedure) };
    assert!(registry.observed_export(&memory, &wrong_weak).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
}

#[test]
fn ole32_terminal_and_drop_revoke_surviving_owning_and_weak_procedures() {
    let mut memory = heap();
    let mut registry = ole32_registry(&memory);
    registry.load_kernel32(&memory).unwrap();
    let module = registry.load_ole32(&memory).unwrap();
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let weak = registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module,
        ProcedureQuery::Name(b"CoUninitialize")).unwrap().unwrap();
    let imported = registry.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    let exit = lease(&mut registry, &memory, Export::ExitProcess);
    let transfer = registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 3).unwrap();
    assert!(registry.require_module(&memory, &module).is_err());
    assert!(registry.procedure_export(&memory, &imported).is_err());
    assert!(registry.observed_export(&memory, &weak).is_err());
    assert!(!imported.module.active.load(Ordering::Acquire));
    assert_eq!(registry.finish(&memory, transfer).unwrap().exit_code, 3);
    let mut later = ole32_registry(&memory);
    later.load_ole32(&memory).unwrap();
    let surviving = later.lease_import(&memory, b"ole32", ProcedureQuery::Name(b"CoUninitialize")).unwrap();
    drop(later);
    assert!(!surviving.module.active.load(Ordering::Acquire));
}

#[test]
fn explicit_configuration_never_auto_loads_or_supports_other_libraries() {
    let memory = heap();
    let mut disabled = configuration();
    disabled.kernel32 = Kernel32Policy::Disabled;
    let mut disabled = Registry::new(&memory, disabled).unwrap();
    assert!(matches!(disabled.load_kernel32(&memory), Err(Stop::Unsupported(_))));
    assert_eq!(disabled.history(&memory).unwrap().modules_issued, 0);
    let mut enabled = Registry::new(&memory, configuration()).unwrap();
    assert!(enabled.lease_import(&memory, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    enabled.load_kernel32(&memory).unwrap();
    assert!(enabled.load_kernel32(&memory).is_err());
    for library in [b"user32.dll".as_slice(), b"ntdll.dll", b"msvcrt", b"missing.dll"] {
        assert!(matches!(enabled.lease_import(&memory, library, ProcedureQuery::Name(b"GlobalAlloc")), Err(Stop::Unsupported(_))));
    }
    assert_eq!(enabled.history(&memory).unwrap().procedure_leases_issued, 0);
    assert_eq!(memory.live_allocations(), 0);
}

#[test]
fn invalid_limits_refuse_before_publication() {
    let memory = heap();
    let limits = configuration().limits;
    for bad in [
        Limits { module_issuances: 0, ..limits },
        Limits { module_issuances: MAX_HISTORY + 1, ..limits },
        Limits { procedure_lease_issuances: 0, ..limits },
        Limits { procedure_lease_issuances: MAX_HISTORY + 1, ..limits },
        Limits { weak_observation_issuances: 0, ..limits },
        Limits { weak_observation_issuances: MAX_HISTORY + 1, ..limits },
        Limits { global_allocation_issuances: 0, ..limits },
        Limits { global_allocation_issuances: MAX_HISTORY + 1, ..limits },
        Limits { max_live_global_bytes: 0, ..limits },
        Limits { max_live_global_bytes: MAX_ALLOCATION_BYTES + 1, ..limits },
        Limits { max_global_bytes: 0, ..limits },
        Limits { max_global_bytes: 4097, ..limits },
        Limits { max_name_bytes: 0, ..limits },
        Limits { max_name_bytes: MAX_NAME_BYTES + 1, ..limits },
    ] {
        assert!(matches!(Registry::new(&memory, Configuration { limits: bad, ..configuration() }), Err(Stop::Unsupported(_))));
    }
    assert_eq!(memory.live_allocations(), 0);
}

#[test]
fn kernel32_lookup_is_case_insensitive_with_optional_default_extension() {
    let memory = heap();
    let mut registry = registry(&memory);
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    for name in [b"kernel32".as_slice(), b"KERNEL32", b"KeRnEl32.DlL", b"kernel32.dll"] {
        let reference = registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(name)).unwrap().unwrap();
        registry.require_module(&memory, &reference).unwrap();
        assert_eq!(Arc::strong_count(registry.kernel32.as_ref().unwrap()), 2);
    }
    assert!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"user32.dll")).unwrap().is_none());
    assert!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"kernel32.exe")).unwrap().is_none());
    assert!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), None).is_err());
}

#[test]
fn invalid_names_are_refusals_not_proven_absences() {
    let memory = heap();
    let mut registry = registry(&memory);
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let module = registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"kernel32")).unwrap().unwrap();
    let before = registry.history(&memory).unwrap();
    let too_long = [b'a'; 65];
    for name in [
        b"".as_slice(), b"kernel32\0.dll", b"kernel32.dll\0", b"dir/kernel32.dll",
        b"dir\\kernel32.dll", b"C:kernel32.dll", b"\xff", b"kernel32\n", b"kernel 32",
        b".", b"..", &too_long,
    ] {
        assert!(matches!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(name)), Err(Stop::Unsupported(_))));
        assert!(matches!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Name(name)), Err(Stop::Unsupported(_))));
        assert!(registry.lease_import(&memory, name, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    }
    assert_eq!(registry.history(&memory).unwrap(), before);
    assert_eq!(memory.live_bytes(), 0);
}

#[test]
fn export_table_has_exactly_five_case_exact_concrete_procedures() {
    let memory = heap();
    let mut registry = registry(&memory);
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let module = registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"kernel32")).unwrap().unwrap();
    assert_eq!(registry.kernel32.as_ref().unwrap().procedures.len(), 5);
    for export in EXPORTS {
        let observation = registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Name(export.name())).unwrap().unwrap();
        assert_eq!(registry.observed_export(&memory, &observation).unwrap(), export);
        let imported = lease(&mut registry, &memory, export);
        assert_eq!(registry.procedure_export(&memory, &imported).unwrap(), export);
    }
    for name in [b"globalalloc".as_slice(), b"GLOBALALLOC", b"LoadLibraryA", b"MissingExport"] {
        assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Name(name)).unwrap().is_none());
        assert!(registry.lease_import(&memory, b"kernel32", ProcedureQuery::Name(name)).is_err());
    }
}

#[test]
fn ordinals_stop_without_issuing_a_procedure_or_observation() {
    let memory = heap();
    let mut registry = registry(&memory);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let module = registry.module_reference();
    let before = registry.history(&memory).unwrap();
    for ordinal in [0, 1, 5, u16::MAX] {
        assert!(matches!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Ordinal(ordinal)), Err(Stop::Unsupported(_))));
        assert!(matches!(registry.lease_import(&memory, b"kernel32", ProcedureQuery::Ordinal(ordinal)), Err(Stop::Unsupported(_))));
    }
    assert_eq!(registry.history(&memory).unwrap(), before);
}

#[test]
fn owning_import_keeps_real_kernel32_resolvable_through_failed_retirement() {
    let memory = heap();
    let mut registry = registry(&memory);
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    let dependency = lease(&mut registry, &memory, Export::GlobalAlloc);
    let weak = registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"KERNEL32.DLL")).unwrap().unwrap();
    assert!(registry.retire_kernel32(&memory).is_err());
    registry.require_module(&memory, &weak).unwrap();
    assert_eq!(registry.procedure_export(&memory, &dependency).unwrap(), Export::GlobalAlloc);
    assert!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"kernel32")).unwrap().is_some());
    drop(dependency);
    assert!(registry.retire_kernel32(&memory).is_err());
    drop(get_module);
    registry.retire_kernel32(&memory).unwrap();
    assert!(registry.require_module(&memory, &weak).is_err());
}

#[test]
fn weak_procedure_calls_are_real_but_do_not_pin_or_upgrade_module_ownership() {
    let mut memory = heap();
    let mut registry = registry(&memory);
    let get_module = observe(&mut registry, &memory, Export::GetModuleHandleA);
    let alloc = observe(&mut registry, &memory, Export::GlobalAlloc);
    let free = observe(&mut registry, &memory, Export::GlobalFree);
    assert_eq!(Arc::strong_count(registry.kernel32.as_ref().unwrap()), 1);
    let weak_module = registry.get_module_handle_a(&memory, ProcedureAccess::Lookup(&get_module), Some(b"Kernel32")).unwrap().unwrap();
    let GlobalAllocReturn::Allocated(owner) = registry.global_alloc(&mut memory, ProcedureAccess::Lookup(&alloc), 0, 17).unwrap() else {
        panic!("unexpected host OOM");
    };
    let pointer = registry.allocation_word(&memory, &owner).unwrap();
    assert_eq!(registry.global_free(&mut memory, ProcedureAccess::Lookup(&free), &owner, pointer).unwrap(), GlobalFreeReturn::Released);
    assert_eq!(memory.live_allocations(), 0);
    assert_eq!(Arc::strong_count(registry.kernel32.as_ref().unwrap()), 1);
    registry.retire_kernel32(&memory).unwrap();
    assert!(registry.require_module(&memory, &weak_module).is_err());
    assert!(registry.global_alloc(&mut memory, ProcedureAccess::Lookup(&alloc), 0, 8).is_err());
}

#[test]
fn retired_module_and_procedure_identities_cannot_revive_on_reload() {
    let memory = heap();
    let mut registry = registry(&memory);
    let old_module = registry.module_reference();
    let old_procedure = observe(&mut registry, &memory, Export::GlobalAlloc);
    registry.retire_kernel32(&memory).unwrap();
    let new_module = registry.load_kernel32(&memory).unwrap();
    registry.require_module(&memory, &new_module).unwrap();
    assert!(registry.require_module(&memory, &old_module).is_err());
    assert!(registry.observed_export(&memory, &old_procedure).is_err());
    let new_procedure = observe(&mut registry, &memory, Export::GlobalAlloc);
    assert_eq!(registry.observed_export(&memory, &new_procedure).unwrap(), Export::GlobalAlloc);
}

#[test]
fn registry_drop_and_replacement_invalidate_surviving_imports_and_weak_refs() {
    let memory = heap();
    let mut original = registry(&memory);
    let import = lease(&mut original, &memory, Export::GetModuleHandleA);
    let weak_module = original.module_reference();
    let weak_procedure = observe(&mut original, &memory, Export::GlobalAlloc);
    let observed_lifetime = Arc::downgrade(&original.lifetime);
    drop(original);
    assert!(observed_lifetime.upgrade().is_none());
    assert!(!import.module.active.load(Ordering::Acquire));
    let mut replacement = registry(&memory);
    assert!(replacement.procedure_export(&memory, &import).is_err());
    assert!(replacement.require_module(&memory, &weak_module).is_err());
    assert!(replacement.observed_export(&memory, &weak_procedure).is_err());
    assert!(replacement.get_module_handle_a(&memory, ProcedureAccess::Import(&import), Some(b"kernel32")).is_err());
}

#[test]
fn registry_drop_invalidates_global_receipts_without_claiming_borrowed_heap_cleanup() {
    let mut memory = heap();
    let mut original = registry(&memory);
    let alloc = lease(&mut original, &memory, Export::GlobalAlloc);
    let global = allocate(&mut original, &mut memory, &alloc, 0, 8);
    let pointer = original.allocation_word(&memory, &global).unwrap();
    drop(original);
    let mut replacement = registry(&memory);
    let free = lease(&mut replacement, &memory, Export::GlobalFree);
    assert!(replacement.allocation_word(&memory, &global).is_err());
    assert!(replacement.global_free(&mut memory, ProcedureAccess::Import(&free), &global, pointer).is_err());
    // Drop has no mutable Memory borrow. Parent must arrange terminal cleanup
    // before dropping/replacing a registry that still owns program globals.
    assert_eq!(memory.allocation_bytes(u64::from(pointer)).unwrap(), &[0; 8]);
    assert_eq!(memory.live_allocations(), 1);
}

#[test]
fn foreign_module_and_procedure_identities_are_not_accepted_with_identical_export_names() {
    let memory = heap();
    let mut first = registry(&memory);
    let mut second = registry(&memory);
    let get_proc = lease(&mut first, &memory, Export::GetProcAddress);
    let foreign_module = second.module_reference();
    let foreign_observation = observe(&mut second, &memory, Export::GetProcAddress);
    assert!(first.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &foreign_module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    let module = first.module_reference();
    assert!(first.get_proc_address(&memory, ProcedureAccess::Lookup(&foreign_observation), &module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    // Negative identity substitution control: equal export tags do not make
    // another module's actual Arc<Procedure> a member of the first table.
    let mixed = ProcedureLease {
        registry: Arc::downgrade(&first.lifetime),
        module: Arc::clone(first.kernel32.as_ref().unwrap()),
        procedure: Arc::clone(second.kernel32.as_ref().unwrap().find(b"GlobalAlloc").unwrap()),
    };
    assert!(first.procedure_export(&memory, &mixed).is_err());
}

#[test]
fn weak_getproc_calls_lookup_without_creating_an_owning_import() {
    let memory = heap();
    let mut registry = registry(&memory);
    let get_proc = observe(&mut registry, &memory, Export::GetProcAddress);
    let module = registry.module_reference();
    let leases_before = registry.history(&memory).unwrap().procedure_leases_issued;
    let observed = registry.get_proc_address(&memory, ProcedureAccess::Lookup(&get_proc), &module, ProcedureQuery::Name(b"GlobalFree")).unwrap().unwrap();
    assert_eq!(registry.observed_export(&memory, &observed).unwrap(), Export::GlobalFree);
    assert_eq!(registry.history(&memory).unwrap().procedure_leases_issued, leases_before);
    assert_eq!(Arc::strong_count(registry.kernel32.as_ref().unwrap()), 1);
    registry.retire_kernel32(&memory).unwrap();
    assert!(registry.observed_export(&memory, &observed).is_err());
}

#[test]
fn equal_address_and_bytes_in_foreign_heap_never_qualify_any_operation() {
    let mut first = heap();
    let mut second = heap();
    let mut registry = registry(&first);
    let get_module = lease(&mut registry, &first, Export::GetModuleHandleA);
    let get_proc = lease(&mut registry, &first, Export::GetProcAddress);
    let alloc = lease(&mut registry, &first, Export::GlobalAlloc);
    let free = lease(&mut registry, &first, Export::GlobalFree);
    let exit = lease(&mut registry, &first, Export::ExitProcess);
    let observation = observe(&mut registry, &first, Export::GlobalAlloc);
    let module = registry.module_reference();
    let global = allocate(&mut registry, &mut first, &alloc, 0, 24);
    let pointer = registry.allocation_word(&first, &global).unwrap();
    let foreign = second.allocate_owner(24).unwrap();
    assert_eq!(u64::from(pointer), foreign.token);
    assert_eq!(first.allocation_bytes(u64::from(pointer)).unwrap(), second.allocation_bytes(foreign.token).unwrap());
    let before = registry.history(&first).unwrap();
    assert!(registry.history(&second).is_err());
    assert!(registry.load_kernel32(&second).is_err());
    assert!(registry.retire_kernel32(&second).is_err());
    assert!(registry.lease_import(&second, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert!(registry.procedure_export(&second, &alloc).is_err());
    assert!(registry.require_module(&second, &module).is_err());
    assert!(registry.observed_export(&second, &observation).is_err());
    assert!(registry.get_module_handle_a(&second, ProcedureAccess::Import(&get_module), Some(b"kernel32")).is_err());
    assert!(registry.get_proc_address(&second, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert!(registry.allocation_word(&second, &global).is_err());
    assert!(registry.global_alloc(&mut second, ProcedureAccess::Import(&alloc), 0, 8).is_err());
    assert!(registry.global_free(&mut second, ProcedureAccess::Import(&free), &global, pointer).is_err());
    assert!(registry.exit_process(&mut second, ProcedureAccess::Import(&exit), 0).is_err());
    assert_eq!(registry.history(&first).unwrap(), before);
    assert_eq!(first.live_allocations(), 1);
    assert_eq!(second.live_allocations(), 1);
    assert_eq!(second.allocation_bytes(foreign.token).unwrap(), foreign.data.as_slice());
}

#[test]
fn swapped_same_address_heap_lifetime_is_refused() {
    let mut memory = heap();
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let global = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    let pointer = registry.allocation_word(&memory, &global).unwrap();
    let original = std::mem::replace(&mut memory, heap());
    let foreign = memory.allocate_owner(16).unwrap();
    assert_eq!(foreign.token, u64::from(pointer));
    assert_eq!(original.allocation_bytes(foreign.token).unwrap(), memory.allocation_bytes(foreign.token).unwrap());
    assert!(registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, pointer).is_err());
    assert_eq!(memory.live_allocations(), 1);
    assert_eq!(original.live_allocations(), 1);
    assert_eq!(registry.allocation_word(&original, &global).unwrap(), pointer);
}

#[test]
fn fixed_allocations_are_real_aligned_zeroed_writable_and_actually_freed() {
    let mut memory = heap();
    let unrelated = changed_owner(&mut memory, 0x7b);
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    for flags in [0, 0x40] {
        let owner = allocate(&mut registry, &mut memory, &alloc, flags, 37);
        let pointer = registry.allocation_word(&memory, &owner).unwrap();
        assert_ne!(pointer, 0);
        assert_eq!(pointer % 8, 0);
        assert_eq!(memory.allocation_kind(u64::from(pointer)).unwrap(), Kind::ProgramGlobal);
        assert_eq!(memory.allocation_bytes(u64::from(pointer)).unwrap(), &[0; 37]);
        assert_eq!(memory.live_bytes(), 24 + 37);
        let image = Owner { token: u64::from(pointer), data: vec![0xac; 37] };
        memory.synchronize_owner(&image).unwrap();
        assert_eq!(memory.allocation_bytes(image.token).unwrap(), image.data.as_slice());
        let freed = registry.global_free(&mut memory, ProcedureAccess::Import(&free), &owner, pointer).unwrap();
        assert_eq!(freed, GlobalFreeReturn::Released);
        assert_eq!(freed.return_word(), 0);
        assert!(memory.allocation_bytes(u64::from(pointer)).is_err());
        assert_eq!(memory.live_bytes(), 24);
        assert_eq!(memory.allocation_bytes(unrelated.token).unwrap(), unrelated.data.as_slice());
    }
    assert_eq!(registry.history(&memory).unwrap().global_allocations_issued, 2);
    assert_eq!(registry.history(&memory).unwrap().live_global_bytes, 0);
}

#[test]
fn unsupported_allocation_requests_refuse_before_heap_or_history_effects() {
    let mut memory = heap();
    let unrelated = changed_owner(&mut memory, 0x45);
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let before = registry.history(&memory).unwrap();
    for (flags, bytes) in [(0, 0), (0x40, 0), (2, 8), (0x42, 8), (0x20, 8), (1, 8), (u32::MAX, 8), (0, 1025), (0, u32::MAX)] {
        assert!(matches!(registry.global_alloc(&mut memory, ProcedureAccess::Import(&alloc), flags, bytes), Err(Stop::Unsupported(_))));
    }
    assert_eq!(registry.history(&memory).unwrap(), before);
    assert_eq!(memory.live_bytes(), 24);
    assert_eq!(memory.live_allocations(), 1);
    assert_eq!(memory.allocation_bytes(unrelated.token).unwrap(), unrelated.data.as_slice());
}

#[test]
fn typed_allocator_refusal_never_turns_generic_unavailable_into_null() {
    let refused = Stop::Unavailable("deliberate host profile refusal".into());
    assert_eq!(allocation_error(ProgramAllocationError::Refused(refused.clone())).unwrap_err(), refused);
    let refused = Stop::Unsupported("deliberate unsupported request".into());
    assert_eq!(allocation_error(ProgramAllocationError::Refused(refused.clone())).unwrap_err(), refused);
    assert!(matches!(allocation_error(ProgramAllocationError::OutOfMemory).unwrap(), GlobalAllocReturn::OutOfMemory));
    // This checks classification only; it does not force actual system OOM.
}

#[test]
fn actual_memory_profile_exhaustion_is_host_refusal_with_no_issuance() {
    let mut memory = Memory::new(16).unwrap();
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let before = registry.history(&memory).unwrap();
    assert!(matches!(registry.global_alloc(&mut memory, ProcedureAccess::Import(&alloc), 0, 17), Err(_)));
    assert_eq!(memory.live_allocations(), 0);
    assert_eq!(memory.live_bytes(), 0);
    assert_eq!(registry.history(&memory).unwrap(), before);
    let owner = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    assert_ne!(registry.allocation_word(&memory, &owner).unwrap(), 0);
}

#[test]
fn allocation_history_is_bounded_even_after_successful_free() {
    let mut memory = heap();
    let mut config = configuration();
    config.limits.global_allocation_issuances = 1;
    let mut registry = Registry::new(&memory, config).unwrap();
    registry.load_kernel32(&memory).unwrap();
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let owner = allocate(&mut registry, &mut memory, &alloc, 0, 8);
    let pointer = registry.allocation_word(&memory, &owner).unwrap();
    registry.global_free(&mut memory, ProcedureAccess::Import(&free), &owner, pointer).unwrap();
    assert!(matches!(registry.global_alloc(&mut memory, ProcedureAccess::Import(&alloc), 0, 8), Err(Stop::Unavailable(_))));
    assert_eq!(registry.history(&memory).unwrap().global_allocations_issued, 1);
    assert_eq!(memory.live_allocations(), 0);
}

#[test]
fn live_global_byte_budget_refuses_before_second_allocation_and_recovers_after_free() {
    let mut memory = heap();
    let mut config = configuration();
    config.limits.max_global_bytes = 16;
    config.limits.max_live_global_bytes = 16;
    let mut registry = Registry::new(&memory, config).unwrap();
    registry.load_kernel32(&memory).unwrap();
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let first = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    let pointer = registry.allocation_word(&memory, &first).unwrap();
    assert!(matches!(registry.global_alloc(&mut memory, ProcedureAccess::Import(&alloc), 0, 1), Err(Stop::Unavailable(_))));
    assert_eq!(memory.live_allocations(), 1);
    assert_eq!(registry.history(&memory).unwrap().global_allocations_issued, 1);
    registry.global_free(&mut memory, ProcedureAccess::Import(&free), &first, pointer).unwrap();
    let second = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    assert_ne!(registry.allocation_word(&memory, &second).unwrap(), pointer);
    assert_eq!(memory.live_bytes(), 16);
}

#[test]
fn module_lease_and_weak_histories_refuse_without_publication() {
    let memory = heap();
    let mut config = configuration();
    config.limits.module_issuances = 1;
    config.limits.procedure_lease_issuances = 1;
    config.limits.weak_observation_issuances = 2;
    let mut registry = Registry::new(&memory, config).unwrap();
    let first = registry.load_kernel32(&memory).unwrap();
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    assert!(matches!(registry.lease_import(&memory, b"kernel32", ProcedureQuery::Name(b"GlobalAlloc")), Err(Stop::Unavailable(_))));
    let second = registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"kernel32")).unwrap().unwrap();
    let before = registry.history(&memory).unwrap();
    assert!(matches!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"kernel32")), Err(Stop::Unavailable(_))));
    assert_eq!(registry.history(&memory).unwrap(), before);
    registry.require_module(&memory, &first).unwrap();
    registry.require_module(&memory, &second).unwrap();
    drop(get_module);
    registry.retire_kernel32(&memory).unwrap();
    assert!(matches!(registry.load_kernel32(&memory), Err(Stop::Unavailable(_))));
    assert!(registry.kernel32.is_none());
    assert_eq!(memory.live_allocations(), 0);
}

#[test]
fn procedure_observation_budget_does_not_publish_partial_lookup() {
    let memory = heap();
    let mut config = configuration();
    config.limits.weak_observation_issuances = 1;
    let mut registry = Registry::new(&memory, config).unwrap();
    let module = registry.load_kernel32(&memory).unwrap();
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let before = registry.history(&memory).unwrap();
    assert!(matches!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Name(b"GlobalAlloc")), Err(Stop::Unavailable(_))));
    assert_eq!(registry.history(&memory).unwrap(), before);
    // A real absent export needs no observation slot and remains absent.
    assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module, ProcedureQuery::Name(b"Missing")).unwrap().is_none());
}

#[test]
fn wrong_export_capability_cannot_call_allocation_free_lookup_or_terminal() {
    let mut memory = heap();
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let global = allocate(&mut registry, &mut memory, &alloc, 0, 8);
    let pointer = registry.allocation_word(&memory, &global).unwrap();
    let module = registry.module_reference();
    let before = registry.history(&memory).unwrap();
    assert!(registry.global_alloc(&mut memory, ProcedureAccess::Import(&free), 0, 8).is_err());
    assert!(registry.global_free(&mut memory, ProcedureAccess::Import(&alloc), &global, pointer).is_err());
    assert!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&alloc), Some(b"kernel32")).is_err());
    assert!(registry.get_proc_address(&memory, ProcedureAccess::Import(&alloc), &module, ProcedureQuery::Name(b"GlobalAlloc")).is_err());
    assert!(registry.exit_process(&mut memory, ProcedureAccess::Import(&alloc), 0).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
    assert_eq!(memory.live_allocations(), 1);
}

#[test]
fn free_requires_exact_issued_receipt_and_exact_handle_and_cannot_repeat() {
    let mut memory = heap();
    let unrelated = changed_owner(&mut memory, 0x67);
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let global = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    let pointer = registry.allocation_word(&memory, &global).unwrap();
    let forged = GlobalAllocation { registry: Arc::downgrade(&registry.lifetime), issuance: Arc::new(AllocationIdentity) };
    assert!(registry.global_free(&mut memory, ProcedureAccess::Import(&free), &forged, pointer).is_err());
    for invalid in [0, pointer + 1, pointer + 16, unrelated.token as u32] {
        let result = registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, invalid).unwrap();
        assert_eq!(result, GlobalFreeReturn::Failed { original_handle: invalid, reason: FreeFailure::HandleMismatch });
        assert_eq!(result.return_word(), invalid);
        assert_eq!(memory.live_allocations(), 2);
    }
    assert_eq!(registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, pointer).unwrap(), GlobalFreeReturn::Released);
    let repeat = registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, pointer).unwrap();
    assert_eq!(repeat, GlobalFreeReturn::Failed { original_handle: pointer, reason: FreeFailure::AlreadyReleased });
    assert_eq!(repeat.return_word(), pointer);
    assert!(registry.allocation_word(&memory, &global).is_err());
    assert_eq!(memory.allocation_bytes(unrelated.token).unwrap(), unrelated.data.as_slice());
    assert_eq!(memory.live_allocations(), 1);
}

#[test]
fn foreign_registry_receipts_and_procedures_cannot_free_same_heap_objects() {
    let mut memory = heap();
    let mut first = registry(&memory);
    let mut second = registry(&memory);
    let first_alloc = lease(&mut first, &memory, Export::GlobalAlloc);
    let first_free = lease(&mut first, &memory, Export::GlobalFree);
    let second_alloc = lease(&mut second, &memory, Export::GlobalAlloc);
    let second_free = lease(&mut second, &memory, Export::GlobalFree);
    let a = allocate(&mut first, &mut memory, &first_alloc, 0, 8);
    let b = allocate(&mut second, &mut memory, &second_alloc, 0, 8);
    let ap = first.allocation_word(&memory, &a).unwrap();
    let bp = second.allocation_word(&memory, &b).unwrap();
    assert!(first.global_free(&mut memory, ProcedureAccess::Import(&first_free), &b, bp).is_err());
    assert!(first.global_free(&mut memory, ProcedureAccess::Import(&second_free), &a, ap).is_err());
    assert!(second.global_free(&mut memory, ProcedureAccess::Import(&second_free), &a, ap).is_err());
    let wrong_handle = first.global_free(&mut memory, ProcedureAccess::Import(&first_free), &a, bp).unwrap();
    assert!(matches!(wrong_handle, GlobalFreeReturn::Failed { reason: FreeFailure::HandleMismatch, .. }));
    assert_eq!(memory.live_allocations(), 2);
}

#[test]
fn issued_receipt_cannot_free_equal_pointer_in_a_separate_programglobal_heap() {
    let mut first_memory = heap();
    let mut second_memory = heap();
    let mut first = registry(&first_memory);
    let mut second = registry(&second_memory);
    let a_alloc = lease(&mut first, &first_memory, Export::GlobalAlloc);
    let a_free = lease(&mut first, &first_memory, Export::GlobalFree);
    let b_alloc = lease(&mut second, &second_memory, Export::GlobalAlloc);
    let b_free = lease(&mut second, &second_memory, Export::GlobalFree);
    let a = allocate(&mut first, &mut first_memory, &a_alloc, 0, 16);
    let b = allocate(&mut second, &mut second_memory, &b_alloc, 0, 16);
    let pointer = first.allocation_word(&first_memory, &a).unwrap();
    assert_eq!(second.allocation_word(&second_memory, &b).unwrap(), pointer);
    assert!(first.global_free(&mut second_memory, ProcedureAccess::Import(&a_free), &a, pointer).is_err());
    assert!(second.global_free(&mut second_memory, ProcedureAccess::Import(&b_free), &a, pointer).is_err());
    assert_eq!(first_memory.live_allocations(), 1);
    assert_eq!(second_memory.live_allocations(), 1);
}

#[test]
fn ordinary_world_owner_kind_cannot_be_freed_even_by_corrupt_global_ledger() {
    let mut memory = heap();
    let ordinary = changed_owner(&mut memory, 0x91);
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let global = allocate(&mut registry, &mut memory, &alloc, 0, 24);
    let original_pointer = registry.allocation_word(&memory, &global).unwrap();
    // Negative control corrupts private bookkeeping; no public adoption API
    // permits constructing this mismatch or turning an Owner into a global.
    registry.allocations[0].pointer = ordinary.token as u32;
    let result = registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, ordinary.token as u32).unwrap();
    assert_eq!(result, GlobalFreeReturn::Failed { original_handle: ordinary.token as u32, reason: FreeFailure::WrongAllocationKind });
    assert_eq!(memory.allocation_bytes(ordinary.token).unwrap(), ordinary.data.as_slice());
    assert_eq!(memory.live_allocations(), 2);
    registry.allocations[0].pointer = original_pointer;
    registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, original_pointer).unwrap();
    assert_eq!(memory.live_allocations(), 1);
}

#[test]
fn externally_missing_or_changed_extent_is_host_refusal_not_free_success() {
    let mut memory = heap();
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let global = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    let pointer = registry.allocation_word(&memory, &global).unwrap();
    registry.allocations[0].bytes = 15;
    assert!(registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, pointer).is_err());
    assert_eq!(memory.live_allocations(), 1);
    registry.allocations[0].bytes = 16;
    memory.release(u64::from(pointer), Kind::ProgramGlobal).unwrap();
    assert!(registry.global_free(&mut memory, ProcedureAccess::Import(&free), &global, pointer).is_err());
    assert!(registry.allocations[0].live);
    assert_eq!(memory.live_allocations(), 0);
}

#[test]
fn disabled_exit_policy_refuses_import_and_weak_terminal_call_without_effects() {
    let mut memory = heap();
    let mut config = configuration();
    config.exit = ExitPolicy::Disabled;
    let mut registry = Registry::new(&memory, config).unwrap();
    registry.load_kernel32(&memory).unwrap();
    assert!(registry.lease_import(&memory, b"kernel32", ProcedureQuery::Name(b"ExitProcess")).is_err());
    let exit = observe(&mut registry, &memory, Export::ExitProcess);
    let before = registry.history(&memory).unwrap();
    assert!(registry.exit_process(&mut memory, ProcedureAccess::Lookup(&exit), 0).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
    assert!(registry.lifetime.active.load(Ordering::Acquire));
    assert_eq!(memory.live_allocations(), 0);
}

#[test]
fn terminal_control_releases_only_own_globals_seals_access_and_never_returns_word() {
    let mut memory = heap();
    let unrelated = changed_owner(&mut memory, 0xe4);
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let free = lease(&mut registry, &memory, Export::GlobalFree);
    let exit = lease(&mut registry, &memory, Export::ExitProcess);
    let get_module = lease(&mut registry, &memory, Export::GetModuleHandleA);
    let module = registry.module_reference();
    let observed = observe(&mut registry, &memory, Export::GlobalAlloc);
    let first = allocate(&mut registry, &mut memory, &alloc, 0, 17);
    let second = allocate(&mut registry, &mut memory, &alloc, 0x40, 23);
    let fp = registry.allocation_word(&memory, &first).unwrap();
    let sp = registry.allocation_word(&memory, &second).unwrap();
    // Another registry's genuine ProgramGlobal allocation must survive.
    let mut other = Registry::new(&memory, configuration()).unwrap();
    other.load_kernel32(&memory).unwrap();
    let other_alloc = lease(&mut other, &memory, Export::GlobalAlloc);
    let other_owner = allocate(&mut other, &mut memory, &other_alloc, 0, 13);
    let other_pointer = other.allocation_word(&memory, &other_owner).unwrap();
    let transfer: TerminalTransfer = registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 0x1234_5678).unwrap();
    assert!(memory.allocation_bytes(u64::from(fp)).is_err());
    assert!(memory.allocation_bytes(u64::from(sp)).is_err());
    assert_eq!(memory.live_allocations(), 2);
    assert_eq!(memory.live_bytes(), 24 + 13);
    assert_eq!(memory.allocation_bytes(unrelated.token).unwrap(), unrelated.data.as_slice());
    assert_eq!(other.allocation_word(&memory, &other_owner).unwrap(), other_pointer);
    assert!(registry.history(&memory).is_err());
    assert!(registry.require_module(&memory, &module).is_err());
    assert!(registry.observed_export(&memory, &observed).is_err());
    assert!(registry.procedure_export(&memory, &alloc).is_err());
    assert!(registry.allocation_word(&memory, &first).is_err());
    assert!(registry.get_module_handle_a(&memory, ProcedureAccess::Import(&get_module), Some(b"kernel32")).is_err());
    assert!(registry.global_alloc(&mut memory, ProcedureAccess::Import(&alloc), 0, 8).is_err());
    assert!(registry.global_free(&mut memory, ProcedureAccess::Import(&free), &first, fp).is_err());
    assert!(registry.load_kernel32(&memory).is_err());
    assert!(registry.retire_kernel32(&memory).is_err());
    assert!(registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 2).is_err());
    let termination = registry.finish(&memory, transfer).unwrap();
    assert_eq!(termination, Termination { exit_code: 0x1234_5678, released_globals: 2 });
}

#[test]
fn weak_exit_procedure_is_terminal_without_acquiring_an_import_pin() {
    let mut memory = heap();
    let mut registry = registry(&memory);
    let exit = observe(&mut registry, &memory, Export::ExitProcess);
    assert_eq!(Arc::strong_count(registry.kernel32.as_ref().unwrap()), 1);
    let transfer = registry.exit_process(&mut memory, ProcedureAccess::Lookup(&exit), 9).unwrap();
    assert!(registry.observed_export(&memory, &exit).is_err());
    assert_eq!(registry.finish(&memory, transfer).unwrap(), Termination { exit_code: 9, released_globals: 0 });
}

#[test]
fn terminal_preflight_checks_all_globals_before_any_release_or_seal() {
    let mut memory = heap();
    let mut registry = registry(&memory);
    let alloc = lease(&mut registry, &memory, Export::GlobalAlloc);
    let exit = lease(&mut registry, &memory, Export::ExitProcess);
    let first = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    let second = allocate(&mut registry, &mut memory, &alloc, 0, 16);
    let fp = registry.allocation_word(&memory, &first).unwrap();
    let sp = registry.allocation_word(&memory, &second).unwrap();
    memory.release(u64::from(sp), Kind::ProgramGlobal).unwrap();
    let before = registry.history(&memory).unwrap();
    assert!(registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 1).is_err());
    assert_eq!(registry.history(&memory).unwrap(), before);
    assert!(registry.lifetime.active.load(Ordering::Acquire));
    assert!(registry.terminal.is_none());
    assert_eq!(registry.procedure_export(&memory, &alloc).unwrap(), Export::GlobalAlloc);
    assert_eq!(memory.allocation_bytes(u64::from(fp)).unwrap(), &[0; 16]);
}

#[test]
fn terminal_transfer_requires_exact_registry_and_seal_not_duplicate_code() {
    let mut memory = heap();
    let mut registry = registry(&memory);
    let exit = lease(&mut registry, &memory, Export::ExitProcess);
    let actual = registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 4).unwrap();
    // Only tests can forge a duplicate; the public transfer is non-Clone.
    let duplicate = TerminalTransfer {
        registry: Arc::downgrade(&registry.lifetime),
        terminal: Arc::new(TerminalIdentity { exit_code: 4, released_globals: 0 }),
    };
    assert!(registry.finish(&memory, duplicate).is_err());
    // Test-only duplicate of the *actual* receipt: even identical identity may
    // be consumed only once. Public callers cannot clone TerminalTransfer.
    let replay = TerminalTransfer {
        registry: actual.registry.clone(),
        terminal: Arc::clone(&actual.terminal),
    };
    let stale = TerminalTransfer {
        registry: actual.registry.clone(),
        terminal: Arc::clone(&actual.terminal),
    };
    assert_eq!(registry.finish(&memory, actual).unwrap(), Termination { exit_code: 4, released_globals: 0 });
    assert!(registry.finish(&memory, replay).is_err());
    let mut replacement = Registry::new(&memory, configuration()).unwrap();
    assert!(replacement.finish(&memory, stale).is_err());
    assert!(replacement.lifetime.active.load(Ordering::Acquire));
    assert_eq!(memory.live_allocations(), 0);
}

#[test]
fn terminal_finish_refuses_foreign_heap_even_with_equal_addresses() {
    let mut memory = heap();
    let other = heap();
    let mut registry = registry(&memory);
    let exit = lease(&mut registry, &memory, Export::ExitProcess);
    let transfer = registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 0).unwrap();
    let wrong_heap = TerminalTransfer {
        registry: transfer.registry.clone(),
        terminal: Arc::clone(&transfer.terminal),
    };
    assert!(registry.finish(&other, wrong_heap).is_err());
    assert_eq!(registry.finish(&memory, transfer).unwrap(), Termination { exit_code: 0, released_globals: 0 });
}

// Private Gdi32 program wiring controls. AUTHORED UNRUN; real software owners
// only. These do not qualify Windows defaults, a current screen or guest ABI.
fn gdi_device() -> program_raster::Device {
    program_raster::Device::new(program_raster::Limits {
        max_width: 8, max_height: 8, max_bitmap_bytes: 256,
        max_live_bytes: 4096, max_bitmaps: 64, max_selections: 64,
    }).unwrap()
}
fn gdi_source() -> (program_raster::Device, program_raster::Bitmap) {
    let mut device = gdi_device();
    let bitmap = device.create_bitmap(program_raster::PixelFormat::Rgba8Straight, 2, 1).unwrap();
    let mut selection = device.new_selection().unwrap();
    device.select(&mut selection, &bitmap).unwrap();
    device.fill_rect(&selection, program_raster::Rect { x: 0, y: 0, width: 2, height: 1 },
        program_raster::Pixel::Rgba([19, 37, 73, 151])).unwrap();
    drop(selection);
    (device, bitmap)
}
fn gdi_enable(registry: &mut Registry, memory: &Memory, device: program_raster::Device) {
    registry.enable_private_raster(memory, device, program_raster_context::Policy::PrivateRasterV1, 16).unwrap();
    registry.load_module(memory, ModuleKind::Gdi32).unwrap();
}
fn gdi_lease(registry: &mut Registry, memory: &Memory) -> ProcedureLease {
    registry.lease_import(memory, b"GDI32.dll", ProcedureQuery::Name(b"CreateCompatibleDC")).unwrap()
}

#[test]
fn gdi_program_defaults_and_enable_refusals_preserve_actual_device() {
    let memory = heap();
    for mut registry in [Registry::new(&memory, configuration()).unwrap(),
        Registry::new_with_ole32(&memory, configuration(), program_ole32::Policy::OwnedRustSta).unwrap(),
        Registry::new_with_profiles(&memory, configuration(), program_ole32::Policy::Disabled,
            program_winmm::Policy::Disabled).unwrap()]
    {
        let before = registry.history(&memory).unwrap();
        assert!(registry.dependency_kind(&memory, b"GDI32.dll").is_err());
        assert!(registry.load_module(&memory, ModuleKind::Gdi32).is_err());
        assert!(registry.lease_import(&memory, b"GDI32", ProcedureQuery::Name(b"CreateCompatibleDC")).is_err());
        assert_eq!(registry.history(&memory).unwrap(), before);
    }
    let mut registry = Registry::new(&memory, configuration()).unwrap();
    for (policy, limit) in [(program_raster_context::Policy::Disabled, 16),
        (program_raster_context::Policy::PrivateRasterV1, 0),
        (program_raster_context::Policy::PrivateRasterV1, 4097)]
    {
        let (device, bitmap) = gdi_source();
        let observation = device.observe(&bitmap).unwrap(); let before = device.snapshot().unwrap();
        let failure = registry.enable_private_raster(&memory, device, policy, limit).unwrap_err();
        assert!(failure.device.observe(&bitmap).unwrap().same_bitmap(&observation));
        assert_eq!(failure.device.snapshot().unwrap(), before);
        assert!(registry.private_raster.is_none());
    }
    let foreign_heap = heap();
    let (device, bitmap) = gdi_source();
    let observation = device.observe(&bitmap).unwrap(); let before = device.snapshot().unwrap();
    let failure = registry.enable_private_raster(&foreign_heap, device,
        program_raster_context::Policy::PrivateRasterV1, 16).unwrap_err();
    assert!(failure.device.observe(&bitmap).unwrap().same_bitmap(&observation));
    assert_eq!(failure.device.snapshot().unwrap(), before);
    let (device, bitmap) = gdi_source();
    gdi_enable(&mut registry, &memory, device);
    let enabled = registry.private_raster_observation(&memory).unwrap();
    let (device, other) = gdi_source();
    let observation = device.observe(&other).unwrap(); let before = device.snapshot().unwrap();
    let failure = registry.enable_private_raster(&memory, device,
        program_raster_context::Policy::PrivateRasterV1, 16).unwrap_err();
    assert!(failure.device.observe(&other).unwrap().same_bitmap(&observation));
    assert_eq!(failure.device.snapshot().unwrap(), before);
    assert_eq!(registry.private_raster_observation(&memory).unwrap(), enabled);
    assert_eq!(registry.private_raster.as_ref().unwrap().bitmap_info(&memory, &bitmap).unwrap().width, 2);
}

#[test]
fn gdi_program_real_pixels_independent_defaults_close_and_selected_backing_lifetime() {
    let memory = heap(); let mut registry = Registry::new(&memory, configuration()).unwrap();
    let (device, bitmap) = gdi_source(); gdi_enable(&mut registry, &memory, device);
    let procedure = gdi_lease(&mut registry, &memory);
    let source = registry.raster_attach_source(&memory, &bitmap).unwrap();
    let source_selection = registry.private_raster.as_ref().unwrap().selected_bitmap(&memory, &source).unwrap();
    registry.private_raster.as_mut().unwrap().set_brush(&memory, &source, [7, 11, 13, 17]).unwrap();
    let derived = registry.create_compatible_dc(&memory, ProcedureAccess::Import(&procedure), Some(&source)).unwrap();
    let raster = registry.private_raster.as_ref().unwrap();
    assert_eq!(raster.state(&memory, &derived).unwrap().brush, [255; 4]);
    assert!(!raster.selected_bitmap(&memory, &derived).unwrap().same_bitmap(&source_selection));
    let pixels = registry.raster_readback_rgba(&memory, &source).unwrap();
    assert_eq!((pixels.width, pixels.height), (2, 1));
    assert_eq!(pixels.pixels, [19, 37, 73, 151].repeat(2));
    let pixels = registry.raster_readback_rgba(&memory, &derived).unwrap();
    assert_eq!((pixels.width, pixels.height, pixels.pixels), (1, 1, vec![0, 0, 0, 255]));
    assert_eq!(registry.private_raster_observation(&memory).unwrap().device.live_bitmap_bytes, 12);
    registry.raster_close(&memory, &source).unwrap();
    assert!(registry.raster_close(&memory, &source).is_err());
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&procedure), Some(&source)).is_err());
    // Close released its default/selection, while the actual caller Bitmap lives.
    assert_eq!(registry.private_raster.as_ref().unwrap().bitmap_info(&memory, &bitmap).unwrap().width, 2);
    let attached = registry.raster_attach_source(&memory, &bitmap).unwrap();
    drop(bitmap);
    assert_eq!(registry.raster_readback_rgba(&memory, &attached).unwrap().pixels, [19, 37, 73, 151].repeat(2));
    registry.raster_close(&memory, &attached).unwrap();
    assert_eq!(registry.private_raster_observation(&memory).unwrap().device.live_bitmap_bytes, 2);
    assert_eq!(registry.raster_readback_rgba(&memory, &derived).unwrap().pixels, [0, 0, 0, 255]);
    drop(derived);
    let after = registry.private_raster_observation(&memory).unwrap();
    assert_eq!((after.live_contexts, after.device.live_bitmap_bytes), (0, 0));
}

#[test]
fn gdi_program_authentic_tables_none_foreign_heap_context_and_thread_refuse() {
    let memory = heap(); let foreign_heap = heap();
    let mut registry = registry(&memory); let mut foreign = Registry::new(&memory, configuration()).unwrap();
    let (device, bitmap) = gdi_source(); gdi_enable(&mut registry, &memory, device);
    let (device, other_bitmap) = gdi_source(); gdi_enable(&mut foreign, &memory, device);
    let source = registry.raster_attach_source(&memory, &bitmap).unwrap();
    let other_context = foreign.raster_attach_source(&memory, &other_bitmap).unwrap();
    let procedure = gdi_lease(&mut registry, &memory); let foreign_procedure = gdi_lease(&mut foreign, &memory);
    let wrong = lease(&mut registry, &memory, Export::GlobalFree);
    assert_eq!(registry.gdi32.as_ref().unwrap().procedures.len(), 1);
    assert_eq!(registry.procedure_export(&memory, &procedure).unwrap(), Export::CreateCompatibleDC);
    for name in [b"gdi32".as_slice(), b"GDI32.DLL", b"GdI32"] {
        assert_eq!(registry.dependency_kind(&memory, name).unwrap(), ModuleKind::Gdi32);
    }
    let before = registry.private_raster_observation(&memory).unwrap();
    let history = registry.history(&memory).unwrap();
    for query in [ProcedureQuery::Name(b"DeleteDC"), ProcedureQuery::Name(b"createcompatibledc"),
        ProcedureQuery::Name(b"CreateCompatibleDC\0"), ProcedureQuery::Ordinal(1)]
    { assert!(registry.lease_import(&memory, b"gdi32", query).is_err()); }
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&procedure), None).is_err());
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&foreign_procedure), Some(&source)).is_err());
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&wrong), Some(&source)).is_err());
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&procedure), Some(&other_context)).is_err());
    assert!(registry.raster_attach_source(&memory, &other_bitmap).is_err());
    assert!(registry.create_compatible_dc(&foreign_heap, ProcedureAccess::Import(&procedure), Some(&source)).is_err());
    assert!(registry.raster_readback_rgba(&memory, &other_context).is_err());
    assert!(registry.raster_close(&memory, &other_context).is_err());
    assert!(registry.raster_readback_rgba(&foreign_heap, &source).is_err());
    assert!(registry.raster_close(&foreign_heap, &source).is_err());
    // Even the correct export spelling in another actual table is no authority.
    let mixed = ProcedureLease { registry: Arc::downgrade(&registry.lifetime),
        module: Arc::clone(registry.gdi32.as_ref().unwrap()), procedure: Arc::clone(&foreign_procedure.procedure) };
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&mixed), Some(&source)).is_err());
    std::thread::scope(|scope| {
        scope.spawn(|| {
            assert!(registry.raster_attach_source(&memory, &bitmap).is_err());
            assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&procedure), Some(&source)).is_err());
            assert!(registry.raster_readback_rgba(&memory, &source).is_err());
            assert!(registry.raster_close(&memory, &source).is_err());
            assert!(registry.private_raster_observation(&memory).is_err());
        }).join().unwrap();
    });
    assert_eq!(registry.history(&memory).unwrap(), history);
    assert_eq!(registry.private_raster_observation(&memory).unwrap(), before);
}

#[test]
fn gdi_program_retirement_reload_drop_and_terminal_revoke_genuine_context_owners() {
    let mut memory = heap(); let mut registry = registry(&memory);
    let (device, bitmap) = gdi_source(); gdi_enable(&mut registry, &memory, device);
    let procedure = gdi_lease(&mut registry, &memory);
    let get_proc = lease(&mut registry, &memory, Export::GetProcAddress);
    let module = registry.module_reference_for(ModuleKind::Gdi32);
    let old_id = registry.module_identity(&memory, &module).unwrap();
    let weak = registry.get_proc_address(&memory, ProcedureAccess::Import(&get_proc), &module,
        ProcedureQuery::Name(b"CreateCompatibleDC")).unwrap().unwrap();
    let source = registry.raster_attach_source(&memory, &bitmap).unwrap();
    drop(procedure);
    assert!(registry.retire_module(&memory, ModuleKind::Gdi32).is_err());
    registry.raster_close(&memory, &source).unwrap();
    registry.retire_module(&memory, ModuleKind::Gdi32).unwrap();
    assert!(registry.raster_attach_source(&memory, &bitmap).is_err());
    let new_module = registry.load_module(&memory, ModuleKind::Gdi32).unwrap();
    assert_ne!(registry.module_identity(&memory, &new_module).unwrap(), old_id);
    assert!(registry.require_module(&memory, &module).is_err());
    let source = registry.raster_attach_source(&memory, &bitmap).unwrap();
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Lookup(&weak), Some(&source)).is_err());
    let fresh = gdi_lease(&mut registry, &memory);
    let derived = registry.create_compatible_dc(&memory, ProcedureAccess::Import(&fresh), Some(&source)).unwrap();
    let exit = lease(&mut registry, &memory, Export::ExitProcess);
    let transfer = registry.exit_process(&mut memory, ProcedureAccess::Import(&exit), 31).unwrap();
    assert!(registry.private_raster.is_none());
    assert!(registry.raster_readback_rgba(&memory, &source).is_err());
    assert!(registry.raster_close(&memory, &derived).is_err());
    assert!(registry.procedure_export(&memory, &fresh).is_err());
    let (device, other) = gdi_source(); let observed = device.observe(&other).unwrap();
    let failure = registry.enable_private_raster(&memory, device,
        program_raster_context::Policy::PrivateRasterV1, 16).unwrap_err();
    assert!(failure.device.observe(&other).unwrap().same_bitmap(&observed));
    assert_eq!(registry.finish(&memory, transfer).unwrap().exit_code, 31);
    drop(registry); drop(source); drop(derived);
    // A surviving Bitmap cannot be adopted into a replacement Device/registry.
    let mut replacement = Registry::new(&memory, configuration()).unwrap();
    gdi_enable(&mut replacement, &memory, gdi_device());
    assert!(replacement.raster_attach_source(&memory, &bitmap).is_err());
    let (device, bitmap) = gdi_source();
    let mut owner = Registry::new(&memory, configuration()).unwrap();
    gdi_enable(&mut owner, &memory, device);
    let context = owner.raster_attach_source(&memory, &bitmap).unwrap();
    let observed = owner.private_raster.as_ref().unwrap().observe(&memory, &context).unwrap();
    drop(owner);
    assert!(replacement.raster_readback_rgba(&memory, &context).is_err());
    assert!(replacement.private_raster.as_ref().unwrap().observation_live(&memory, &observed).is_err());
    drop(context);
}

#[test]
fn gdi_program_context_history_remains_bounded_after_close_and_resource_refusal() {
    let memory = heap(); let mut registry = Registry::new(&memory, configuration()).unwrap();
    let (device, bitmap) = gdi_source();
    registry.enable_private_raster(&memory, device, program_raster_context::Policy::PrivateRasterV1, 1).unwrap();
    registry.load_module(&memory, ModuleKind::Gdi32).unwrap();
    let procedure = gdi_lease(&mut registry, &memory);
    let context = registry.raster_attach_source(&memory, &bitmap).unwrap();
    let before = registry.private_raster_observation(&memory).unwrap();
    assert!(registry.create_compatible_dc(&memory, ProcedureAccess::Import(&procedure), Some(&context)).is_err());
    assert_eq!(registry.private_raster_observation(&memory).unwrap(), before);
    registry.raster_close(&memory, &context).unwrap();
    let closed = registry.private_raster_observation(&memory).unwrap();
    assert!(registry.raster_attach_source(&memory, &bitmap).is_err());
    assert_eq!(registry.private_raster_observation(&memory).unwrap(), closed);
    assert_eq!((closed.contexts_issued, closed.live_contexts), (1, 0));

    let mut device = program_raster::Device::new(program_raster::Limits {
        max_width: 2, max_height: 1, max_bitmap_bytes: 8, max_live_bytes: 16,
        max_bitmaps: 1, max_selections: 4,
    }).unwrap();
    let bitmap = device.create_bitmap(program_raster::PixelFormat::Rgba8Straight, 2, 1).unwrap();
    let mut limited = Registry::new(&memory, configuration()).unwrap();
    limited.enable_private_raster(&memory, device, program_raster_context::Policy::PrivateRasterV1, 1).unwrap();
    limited.load_module(&memory, ModuleKind::Gdi32).unwrap();
    assert!(limited.raster_attach_source(&memory, &bitmap).is_err());
    let failed = limited.private_raster_observation(&memory).unwrap();
    assert_eq!((failed.contexts_issued, failed.live_contexts, failed.device.live_bitmap_bytes), (1, 0, 8));
    assert_eq!((failed.device.bitmap_issuances, failed.device.selection_issuances), (1, 1));
    // The failed attempt consumed its real history once A effects began. There
    // is no rollback/adoption or second device attempt after B is exhausted.
    assert!(limited.raster_attach_source(&memory, &bitmap).is_err());
    assert_eq!(limited.private_raster_observation(&memory).unwrap(), failed);
}
