use super::*;
use sims_sim::native_application::{Call, Construction, Cut, Host as _, Incoming, SectionRequest};
#[cfg(target_os = "macos")]
mod fpu_tests;
#[cfg(target_os = "macos")]
mod mutex_tests;

fn input(host: &mut BootstrapServices<'_>) -> Incoming {
    let mut app = host.memory.allocate_owner(324).unwrap();
    app.data.fill(0x73);
    let mut argument = host.memory.allocate_owner(16).unwrap();
    argument.data[..5].copy_from_slice(b"Sims\0");
    let mut caller = host.memory.allocate_owner(96).unwrap();
    caller.data[2] = 0x7a;
    caller.data[6] = 0x59;
    caller.data[68..72].copy_from_slice(&(argument.token as u32).to_le_bytes());
    let mut nt_name = host.memory.allocate_owner(32).unwrap();
    let mut local_name = host.memory.allocate_owner(32).unwrap();
    nt_name.data[..12].copy_from_slice(b"Global\\Sims\0");
    local_name.data[..5].copy_from_slice(b"Sims\0");
    for owner in [&app, &argument, &caller, &nt_name, &local_name] {
        host.memory.synchronize_owner(owner).unwrap();
    }
    Incoming {
        application: app,
        argument: argument.token,
        tree_stack: [caller.token + 2, caller.token + 6],
        mutex_temporary: caller.token + 16,
        mutex_name_globals: Owner {
            token: 0x6c9f70,
            data: [nt_name.token as u32, local_name.token as u32]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect(),
        },
        mutex_singleton: Owner {
            token: 0x6f2a5c,
            data: 0x11223344_u32.to_le_bytes().to_vec(),
        },
        retained: vec![argument, caller, nt_name, local_name],
        registry: Default::default(),
        // Explicit synthetic owned test-static input; not a native registry epoch.
        registry_globals: Owner {
            token: 0x6de858,
            data: vec![0; 8],
        },
        application_singleton: Owner {
            token: 0x6f2a58,
            data: 0xaabbccdd_u32.to_le_bytes().to_vec(),
        },
    }
}

fn finish(c: &mut Construction, host: &mut BootstrapServices<'_>) {
    for _ in 0..4096 {
        if c.step(&mut host.application_host()).unwrap() == Some(Cut::ImportEntry) {
            return;
        }
    }
    panic!("application prefix did not reach CoInitialize");
}

#[test]
fn real_section_heap_and_lock_allow_the_same_application_prefix_to_advance() {
    let (index, dirs) = mounted(&[]);
    let mut host = services(&index, &dirs);
    let mut c = Construction::new(input(&mut host)).unwrap();
    finish(&mut c, &mut host);
    c.validate().unwrap();
    let section = c.section().unwrap();
    section
        .validate(&SectionRequest {
            instruction: 0x5d91bc,
            import_slot: 0x666114,
            region: section.region.clone(),
        })
        .unwrap();
    let debug = section.debug_info.as_ref().unwrap();
    assert_eq!(
        host.memory.allocation_bytes(debug.token).unwrap(),
        debug.data
    );
    assert_eq!(
        host.section_debug_info(section.capability, section.region.token, 24)
            .unwrap(),
        Some(debug)
    );
    assert_eq!(
        u32::from_le_bytes(section.region.data[..4].try_into().unwrap()) as u64,
        debug.token
    );
    assert_eq!(&section.region.data[4..8], &u32::MAX.to_le_bytes());
    let lock = Arc::clone(
        host.section(section.capability, section.region.token, 24)
            .unwrap(),
    );
    let guard = lock.lock().unwrap();
    assert!(
        host.section(section.capability, section.region.token, 24)
            .unwrap()
            .try_lock()
            .is_err()
    );
    drop(guard);
    assert!(
        host.section(section.capability, section.region.token, 24)
            .unwrap()
            .try_lock()
            .is_ok()
    );
    assert!(names::Host::free_string(&mut host, debug).is_err());
    assert!(host.memory.release_owner(debug.token).is_err());
    assert!(path::Host::close_scenario(&mut host, section.capability).is_err());
    let before = (
        host.memory.live_bytes(),
        host.memory.live_allocations(),
        host.live_capabilities(),
    );
    let request = SectionRequest {
        instruction: 0x5d91bc,
        import_slot: 0x666114,
        region: section.region.clone(),
    };
    assert!(
        host.application_host()
            .initialize_section(&request)
            .is_err()
    );
    assert_eq!(
        (
            host.memory.live_bytes(),
            host.memory.live_allocations(),
            host.live_capabilities()
        ),
        before
    );
    assert!(matches!(c.pending_call(), Some(Call::CoInitialize(_))));
    let owner = c.owner().clone();
    host.memory.synchronize_owner(&owner).unwrap();
    assert_eq!(
        &host.memory.allocation_bytes(owner.token).unwrap()[24..48],
        section.region.data
    );
    for _ in 0..16 {
        if matches!(c.pending_call(), Some(Call::GetVersion(_))) {
            break;
        }
        c.step(&mut host.application_host()).unwrap();
    }
    assert_eq!(
        c.application_singleton().data,
        (c.owner().token as u32).to_le_bytes()
    );
    assert_eq!(c.com_initialization().unwrap().hresult(), 0);
    host.apartment(c.com_initialization().unwrap())
        .unwrap()
        .check()
        .unwrap();
    assert!(matches!(c.pending_call(), Some(Call::GetVersion(_))));
    assert!(c.step(&mut host.application_host()).is_err());
    c.validate().unwrap();
    assert_eq!(host.live_capabilities(), 2);
    assert!(c.into_prefix().is_ok());
}

#[test]
fn section_quota_failure_keeps_the_call_pending_without_fabricating_null() {
    let (index, dirs) = mounted(&[]);
    let mut host =
        BootstrapServices::new(Memory::new(632).unwrap(), &index, &dirs, LocalePolicy::C);
    let mut c = Construction::new(input(&mut host)).unwrap();
    let blocker = host.memory.allocate_owner(128).unwrap();
    while !matches!(c.pending_call(), Some(Call::InitializeSection(_))) {
        c.step(&mut host.application_host()).unwrap();
    }
    let before = c.owner().clone();
    let writes = c.writes().to_vec();
    for _ in 0..2 {
        assert!(matches!(
            c.step(&mut host.application_host()),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(c.owner(), &before);
        assert_eq!(c.writes(), writes);
        assert!(c.section().is_none());
        c.validate().unwrap();
        assert_eq!(
            (host.memory.live_bytes(), host.live_capabilities()),
            (628, 0)
        );
    }
    host.memory.release_owner(blocker.token).unwrap();
    finish(&mut c, &mut host);
    assert!(c.section().unwrap().debug_info.is_some());
    assert_eq!(host.live_capabilities(), 1);
}

#[test]
fn capability_exhaustion_precedes_debug_allocation_and_metadata_mutation_is_detected() {
    let (index, dirs) = mounted(&[]);
    let mut host = services(&index, &dirs);
    let mut c = Construction::new(input(&mut host)).unwrap();
    while !matches!(c.pending_call(), Some(Call::InitializeSection(_))) {
        c.step(&mut host.application_host()).unwrap();
    }
    host.next = CAP_START + MAX_CAPABILITIES;
    let before = (host.memory.live_bytes(), host.memory.live_allocations());
    assert!(c.step(&mut host.application_host()).is_err());
    assert_eq!(
        (host.memory.live_bytes(), host.memory.live_allocations()),
        before
    );
    assert_eq!(host.live_capabilities(), 0);
    // Restore the artificial test quota; normal production capability IDs never rewind.
    host.next = CAP_START;
    finish(&mut c, &mut host);
    let section = c.section().unwrap();
    let mut debug = section.debug_info.as_ref().unwrap().clone();
    debug.data[4] ^= 1;
    host.memory.synchronize_owner(&debug).unwrap();
    assert!(
        host.section(section.capability, section.region.token, 24)
            .is_err()
    );
    assert!(
        host.section_debug_info(section.capability, section.region.token, 24)
            .is_err()
    );
}

#[test]
fn section_lookups_reject_a_released_containing_application_owner() {
    let (index, dirs) = mounted(&[]);
    let mut host = services(&index, &dirs);
    let mut c = Construction::new(input(&mut host)).unwrap();
    finish(&mut c, &mut host);
    let section = c.section().unwrap();
    let cap = section.capability;
    let address = section.region.token;
    let debug = section.debug_info.as_ref().unwrap().token;
    host.memory.release_owner(c.owner().token).unwrap();
    assert!(
        host.memory.allocation_bytes(debug).is_ok(),
        "debug storage remains retained for exact lifecycle handling"
    );
    assert!(host.section(cap, address, 24).is_err());
    assert!(host.section_debug_info(cap, address, 24).is_err());
    assert_eq!(
        host.live_capabilities(),
        1,
        "rejection does not silently drop the retained capability"
    );
}

#[test]
fn apartment_authority_is_shared_by_thread_but_bound_to_its_issuing_services_and_owner() {
    let (index, dirs) = mounted(&[]);
    let mut first = services(&index, &dirs);
    let mut second = services(&index, &dirs);
    let mut a = Construction::new(input(&mut first)).unwrap();
    let mut b = Construction::new(input(&mut second)).unwrap();
    assert_eq!(a.owner().token, b.owner().token);
    for (c, host) in [(&mut a, &mut first), (&mut b, &mut second)] {
        finish(c, host);
        while !matches!(c.pending_call(), Some(Call::GetVersion(_))) {
            c.step(&mut host.application_host()).unwrap();
        }
    }
    let ar = a.com_initialization().unwrap();
    let br = b.com_initialization().unwrap();
    assert_eq!((ar.hresult(), br.hresult()), (0, 1));
    assert_eq!(first.apartment(ar).unwrap().initialization_count(), 2);
    assert!(first.apartment(br).is_err());
    assert!(second.apartment(ar).is_err());
    first.memory.release_owner(a.owner().token).unwrap();
    assert!(first.apartment(ar).is_err());
    assert_eq!(first.live_capabilities(), 2);
    drop(a);
    // Retaining the backend still retains its one initialization obligation.
    assert_eq!(second.apartment(br).unwrap().initialization_count(), 2);
    drop(first);
    assert_eq!(second.apartment(br).unwrap().initialization_count(), 1);
}

#[test]
fn com_capability_exhaustion_preserves_pending_cut_and_has_no_apartment_effect() {
    let (index, dirs) = mounted(&[]);
    let mut host = services(&index, &dirs);
    let mut c = Construction::new(input(&mut host)).unwrap();
    finish(&mut c, &mut host);
    host.next = CAP_START + MAX_CAPABILITIES;
    let before = c.application_singleton().clone();
    for _ in 0..2 {
        assert!(matches!(
            c.step(&mut host.application_host()),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(c.cut(), Some(Cut::ImportEntry));
        assert_eq!(c.application_singleton(), &before);
        c.validate().unwrap();
        assert_eq!(host.live_capabilities(), 1);
    }
    // Only this test rewinds its deliberately exhausted quota.
    host.next = CAP_START + 1;
    while !matches!(c.pending_call(), Some(Call::GetVersion(_))) {
        c.step(&mut host.application_host()).unwrap();
    }
    assert_eq!(c.com_initialization().unwrap().hresult(), 0);
    assert_eq!(host.live_capabilities(), 2);
}

#[test]
fn heap_replacement_rejects_old_apartment_and_section_capabilities() {
    let (index, dirs) = mounted(&[]);
    let mut first = services(&index, &dirs);
    let mut second = services(&index, &dirs);
    let mut a = Construction::new(input(&mut first)).unwrap();
    let mut b = Construction::new(input(&mut second)).unwrap();
    for (c, host) in [(&mut a, &mut first), (&mut b, &mut second)] {
        finish(c, host);
        while !matches!(c.pending_call(), Some(Call::GetVersion(_))) {
            c.step(&mut host.application_host()).unwrap();
        }
    }
    let section = a.section().unwrap();
    assert_eq!(a.owner().token, b.owner().token);
    assert_eq!(
        first.memory().allocation_bytes(a.owner().token).unwrap(),
        second.memory().allocation_bytes(b.owner().token).unwrap()
    );
    assert_eq!(section.debug_info, b.section().unwrap().debug_info);
    assert!(first.apartment(a.com_initialization().unwrap()).is_ok());
    assert!(
        first
            .section(section.capability, section.region.token, 24)
            .is_ok()
    );
    std::mem::swap(first.memory_mut(), second.memory_mut());
    let accepted = [
        first.apartment(a.com_initialization().unwrap()).is_ok(),
        first
            .section(section.capability, section.region.token, 24)
            .is_ok(),
        first
            .section_debug_info(section.capability, section.region.token, 24)
            .is_ok(),
    ];
    assert_eq!(
        accepted,
        [false, false, false],
        "equal addresses, roles and bytes cannot authenticate another heap"
    );
    assert_eq!(
        first.live_capabilities(),
        2,
        "rejection retains exact resources for cleanup"
    );
}

#[test]
fn explicit_version_policy_advances_real_wrapper_and_temporary_heap_to_pending_mutex() {
    for (raw, name) in [
        (5, b"Global\\Sims".as_slice()),
        (0x80000005, b"Sims".as_slice()),
    ] {
        let (index, dirs) = mounted(&[]);
        let mut host = services(&index, &dirs).with_application_version_policy(
            sims_sim::native_application::platform::VersionPolicy::RawDword(raw),
        );
        let mut c = Construction::new(input(&mut host)).unwrap();
        for _ in 0..4096 {
            if matches!(c.pending_call(), Some(Call::CreateMutex(_))) {
                break;
            }
            c.step(&mut host.application_host()).unwrap();
        }
        let Some(Call::CreateMutex(request)) = c.pending_call() else {
            panic!("missing mutex entry")
        };
        assert_eq!(c.version().unwrap().raw(), raw);
        assert_eq!(request.name_bytes().unwrap(), name);
        assert_eq!(
            host.memory.allocation_kind(request.wrapper()).unwrap(),
            Kind::ApplicationMutexWrapper
        );
        assert_eq!(
            host.memory
                .allocation_bytes(request.wrapper())
                .unwrap()
                .len(),
            12
        );
        assert!(c.step(&mut host.application_host()).is_err());
        c.validate().unwrap();
        assert_eq!(
            host.live_capabilities(),
            2,
            "no mutex handle is manufactured at the pending call"
        );
    }
}
