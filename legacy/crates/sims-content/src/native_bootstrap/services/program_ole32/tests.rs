//! Authored UNRUN. Parent must wire the module, compile and execute these.
//! Fixtures use the public Application construction and real issuing services;
//! they neither expose Request::new nor adopt a synthetic successful Return.

use super::super::{BootstrapServices, LocalePolicy};
use super::*;
use crate::{Index, avatar_catalog::Directories, native_bootstrap::memory::Memory};
use sims_sim::{
    native_application::{Call, Construction, Incoming},
    native_world_children::Owner,
};
use std::path::PathBuf;

fn empty_mount() -> (Index, Directories) {
    // No paths are opened: the authored Application prefix uses owned bytes.
    (
        Index {
            root: PathBuf::from("authored-ole32-empty-mount"),
            resources: vec![],
            archives: 0,
            diagnostics: vec![],
        },
        Directories::default(),
    )
}

fn services<'a>(index: &'a Index, directories: &'a Directories) -> BootstrapServices<'a> {
    BootstrapServices::new(
        Memory::new(8 * 1024 * 1024).unwrap(),
        index,
        directories,
        LocalePolicy::C,
    )
}

// Copied from the authored services/tests/application.rs fixture, using public
// Memory accessors. Static images describe input; they confer no COM authority.
fn input(host: &mut BootstrapServices<'_>) -> Incoming {
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

fn initialize(host: &mut BootstrapServices<'_>) -> (Construction, com::Request, com::Return) {
    let mut construction = Construction::new(input(host)).unwrap();
    let mut pending = None;
    for _ in 0..4096 {
        if let Some(Call::CoInitialize(request)) = construction.pending_call() {
            pending = Some(request);
            break;
        }
        construction.step(&mut host.application_host()).unwrap();
    }
    let request = pending.expect("public Application construction must issue its private Request");
    // Let the real cursor invoke the actual issuing service and retain its
    // staged/completed history. The producer does not create a Return itself.
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
    panic!("Application must retain its real CoInitialize return");
}

#[test]
fn disabled_refuses_before_releasing_a_genuine_retained_obligation() {
    let (index, directories) = empty_mount();
    let mut host = services(&index, &directories);
    let (construction, request, returned) = initialize(&mut host);
    let clone = returned.clone();
    let access = host.apartment(&returned).unwrap();
    let before = (
        host.memory().live_bytes(),
        host.memory().live_allocations(),
        host.live_capabilities(),
    );
    assert_eq!(returned.hresult(), 0);
    for _ in 0..2 {
        assert!(matches!(uninitialize(Policy::Disabled, &clone), Err(Stop::Unsupported(_))));
        access.check().unwrap();
        assert_eq!(access.initialization_count(), 1);
        clone.validate(&request).unwrap();
        construction.validate().unwrap();
        assert_eq!(
            (host.memory().live_bytes(), host.memory().live_allocations(), host.live_capabilities()),
            before
        );
    }
    let (): () = uninitialize(Policy::OwnedRustSta, &returned).unwrap();
    assert!(access.check().is_err());
    assert!(host.apartment(&clone).is_err());
    assert!(matches!(uninitialize(Policy::Disabled, &clone), Err(Stop::Unsupported(_))));
    construction.validate().unwrap();
}

#[test]
fn owned_void_release_revokes_all_history_clones_and_preserves_nested_initialization() {
    let (index, directories) = empty_mount();
    let mut host = services(&index, &directories);
    let (first_history, first_request, first) = initialize(&mut host);
    let retained = first.clone();
    let (nested_history, nested_request, nested) = initialize(&mut host);
    assert_eq!((first.hresult(), nested.hresult()), (0, 1));
    let access = first.current_apartment().unwrap();
    assert_eq!(access.initialization_count(), 2);
    let before = (
        host.memory().live_bytes(),
        host.memory().live_allocations(),
        host.live_capabilities(),
    );
    let (): () = uninitialize(Policy::OwnedRustSta, &retained).unwrap();
    assert!(access.check().is_err());
    assert!(first.current_apartment().is_err());
    assert!(host.apartment(&first).is_err());
    assert!(host.apartment(&retained).is_err());
    assert_eq!(host.apartment(&nested).unwrap().initialization_count(), 1);
    for _ in 0..2 {
        assert!(uninitialize(Policy::OwnedRustSta, &first).is_err());
        assert_eq!(host.apartment(&nested).unwrap().initialization_count(), 1);
    }
    let failed = com::Return::failed(&first_request, 0x80010106).unwrap();
    assert!(uninitialize(Policy::OwnedRustSta, &failed).is_err());
    failed.validate(&first_request).unwrap();
    assert_eq!(host.apartment(&nested).unwrap().initialization_count(), 1);
    assert_eq!(
        (host.memory().live_bytes(), host.memory().live_allocations(), host.live_capabilities()),
        before
    );
    first.validate(&first_request).unwrap();
    retained.validate(&first_request).unwrap();
    nested.validate(&nested_request).unwrap();
    first_history.validate().unwrap();
    nested_history.validate().unwrap();
    let (): () = uninitialize(Policy::OwnedRustSta, &nested).unwrap();
    assert!(host.apartment(&nested).is_err());
    nested_history.validate().unwrap();
}

#[test]
fn dropping_released_application_and_service_history_preserves_a_new_generation() {
    let (index, directories) = empty_mount();
    let mut old_host = services(&index, &directories);
    let (old_history, old_request, old) = initialize(&mut old_host);
    let retained = old.clone();
    uninitialize(Policy::OwnedRustSta, &old).unwrap();
    let mut later_host = services(&index, &directories);
    let (later_history, later_request, later) = initialize(&mut later_host);
    assert_eq!(later.hresult(), 0);
    assert!(uninitialize(Policy::OwnedRustSta, &retained).is_err());
    assert_eq!(later_host.apartment(&later).unwrap().initialization_count(), 1);
    old.validate(&old_request).unwrap();
    old_history.validate().unwrap();
    drop(old_history);
    drop(old);
    drop(retained);
    // The issuing service holds another genuine Return clone until this Drop.
    drop(old_host);
    assert_eq!(later_host.apartment(&later).unwrap().initialization_count(), 1);
    later.validate(&later_request).unwrap();
    later_history.validate().unwrap();
    uninitialize(Policy::OwnedRustSta, &later).unwrap();
    assert!(later_host.apartment(&later).is_err());
}
