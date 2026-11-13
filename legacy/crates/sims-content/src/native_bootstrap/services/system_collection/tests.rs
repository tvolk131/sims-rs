//! Authored integration controls, UNRUN. Requires Worker A and the exact parent
//! wiring in system_collection's module documentation. Small authored inputs;
//! actual Memory/BootstrapServices/factory origin, no native or gameplay claim.
use super::super::{CAP_START, LocalePolicy, MAX_CAPABILITIES, system_factory};
use super::*;

#[path = "../system_collection_event/tests.rs"]
mod event_backend_tests;
use crate::{Index, avatar_catalog::Directories};
use sims_sim::native_system::{collection, factory};

fn fixture() -> (Index, Directories) {
    (
        Index {
            root: "authored-collection-prefix".into(),
            resources: vec![],
            archives: 0,
            diagnostics: vec![],
        },
        Directories::default(),
    )
}
fn services<'a>(i: &'a Index, d: &'a Directories) -> BootstrapServices<'a> {
    BootstrapServices::new(Memory::new(8192).unwrap(), i, d, LocalePolicy::C)
}
fn counts(h: &BootstrapServices<'_>) -> (usize, usize, usize, u32) {
    (
        h.memory.live_bytes(),
        h.memory.live_allocations(),
        h.live_capabilities(),
        h.next,
    )
}
fn factory_frontier(h: &mut BootstrapServices<'_>) -> factory::Checkpoint {
    let mut caller = h.memory.allocate_owner(768).unwrap();
    for (at, byte) in caller.data.iter_mut().enumerate() {
        *byte = (at as u8).wrapping_mul(19).wrapping_add(7);
    }
    h.memory.synchronize_owner(&caller).unwrap();
    let b = caller.token;
    let mut cursor = factory::Construction::new(factory::Incoming {
        origin: h.memory.system_origin(),
        caller,
        frames: factory::Frames {
            factory: b + 32,
            derived: b + 80,
            base: b + 128,
            refcount: b + 176,
            child: b + 224,
            base72: b + 288,
        },
    })
    .unwrap_or_else(|e| panic!("authored factory input: {:?}", e.stop));
    for _ in 0..1024 {
        if matches!(
            cursor.pending_call(),
            Some(factory::Call::Collection {
                instruction: 0x5d5879
            })
        ) {
            let cut = cursor.checkpoint().unwrap();
            assert_eq!(cut.writes().last().unwrap().instruction, 0x5d5875);
            system_factory::synchronize(h, &cut).unwrap();
            return cut;
        }
        let request = cursor.request().clone();
        cursor.step(&mut h.system_provider_host(&request)).unwrap();
    }
    panic!("actual factory did not reach the exact collection frontier");
}
fn cursor(h: &mut BootstrapServices<'_>) -> collection::Construction {
    let factory = factory_frontier(h);
    let b = factory.caller().token;
    collection::Construction::new(collection::Incoming {
        factory,
        frames: collection::Frames {
            collection: b + 400,
            list: b + 448,
            section_base: b + 496,
            resource_argument: b + 528,
        },
    })
    .unwrap_or_else(|e| panic!("authored collection input: {:?}", e.stop))
}

// This is the required PARENT handoff modeled explicitly in a test adapter.
// Production wiring must also refresh its factory's shared current-image record
// and use the collection Binding for later origin checks. No shared source is
// edited by this worker and no stale first-child lifetime check is weakened.
fn refresh_first(h: &mut BootstrapServices<'_>, request: &Request) {
    let Some((parent, caller)) = current_images(h, request) else {
        return;
    };
    let parent = parent.clone();
    let caller = caller.clone();
    let token = request.factory().base72().unwrap().section().capability as u32;
    let Some(Capability::SystemBase72(first)) = h.capabilities.get_mut(&token) else {
        panic!("prevalidated first section disappeared")
    };
    first.parent_image = parent;
    first.caller_image = caller;
}

#[derive(Clone)]
enum Pending {
    List(ListRequest),
    Section(SectionCall),
}
fn effect(site: SectionSite) -> usize {
    match site {
        SectionSite::Collection => 0,
        SectionSite::List => 2,
        SectionSite::Resource => 3,
    }
}
struct Adapter<'s, 'a> {
    h: &'s mut BootstrapServices<'a>,
    request: Request,
    hold: Option<usize>,
    pending: Option<Pending>,
    lose: Option<usize>,
}
impl collection::Host for Adapter<'_, '_> {
    fn synchronize_checkpoint(&mut self, cut: &Checkpoint) -> Result<(), Stop> {
        if !self.request.same_call(cut.request()) {
            return Err(unsupported("test host invocation mismatch"));
        }
        let result = synchronize(self.h, cut);
        refresh_first(self.h, &self.request);
        result
    }
    fn allocate_list(&mut self, call: &ListRequest) -> Result<Allocated, Stop> {
        if !self.request.same_call(call.checkpoint().request()) {
            return Err(unsupported("test list invocation mismatch"));
        }
        if self.hold == Some(1) {
            self.pending = Some(Pending::List(call.clone()));
            return Err(Stop::Unavailable(
                "authored list hold before allocation".into(),
            ));
        }
        let result = allocate_list(self.h, call);
        refresh_first(self.h, &self.request);
        let result = result?;
        if self.lose == Some(1) {
            self.lose = None;
            return Err(Stop::Unavailable(
                "authored lost list reply after retained effect".into(),
            ));
        }
        Ok(result)
    }
    fn initialize_section(&mut self, call: &SectionCall) -> Result<InitializedSection, Stop> {
        if !self.request.same_call(call.checkpoint().request()) {
            return Err(unsupported("test section invocation mismatch"));
        }
        let at = effect(call.site());
        if self.hold == Some(at) {
            self.pending = Some(Pending::Section(call.clone()));
            return Err(Stop::Unavailable(
                "authored section hold before allocation".into(),
            ));
        }
        let result = initialize_section(self.h, call);
        refresh_first(self.h, &self.request);
        let result = result?;
        if self.lose == Some(at) {
            self.lose = None;
            return Err(Stop::Unavailable(
                "authored lost section reply after retained effect".into(),
            ));
        }
        Ok(result)
    }
}
fn step(
    c: &mut collection::Construction,
    h: &mut BootstrapServices<'_>,
) -> Result<Option<collection::Cut>, Stop> {
    let request = c.request().clone();
    c.step(&mut Adapter {
        h,
        request,
        hold: None,
        pending: None,
        lose: None,
    })
}
fn finish(c: &mut collection::Construction, h: &mut BootstrapServices<'_>) -> Checkpoint {
    for _ in 0..1024 {
        if matches!(step(c, h).unwrap(), Some(collection::Cut::BeforeResource)) {
            let cut = c.checkpoint().unwrap();
            cut.validate().unwrap();
            return cut;
        }
    }
    panic!("collection did not stop before resource666238");
}
fn capture(c: &mut collection::Construction, h: &mut BootstrapServices<'_>, at: usize) -> Pending {
    let request = c.request().clone();
    let mut host = Adapter {
        h,
        request,
        hold: Some(at),
        pending: None,
        lose: None,
    };
    for _ in 0..1024 {
        if c.step(&mut host).is_err() {
            return host
                .pending
                .take()
                .expect("only the requested typed source call was held");
        }
    }
    panic!("requested source call was not reached");
}
fn issue(h: &mut BootstrapServices<'_>, p: &Pending) -> Result<(), Stop> {
    match p {
        Pending::List(call) => {
            let result = allocate_list(h, call);
            refresh_first(h, call.checkpoint().request());
            result?;
        }
        Pending::Section(call) => {
            let result = initialize_section(h, call);
            refresh_first(h, call.checkpoint().request());
            result?;
        }
    }
    Ok(())
}
fn cut(p: &Pending) -> &Checkpoint {
    match p {
        Pending::List(call) => call.checkpoint(),
        Pending::Section(call) => call.checkpoint(),
    }
}
fn word(owner: &Owner, offset: usize) -> u32 {
    u32::from_le_bytes(owner.data[offset..offset + 4].try_into().unwrap())
}
fn lookups(h: &BootstrapServices<'_>, cut: &Checkpoint) -> Result<(), Stop> {
    for section in cut.sections().iter().flatten() {
        h.section(section.capability, section.region.token, 24)?;
        assert_eq!(
            h.section_debug_info(section.capability, section.region.token, 24)?,
            section.debug_info.as_ref()
        );
    }
    let first = cut.request().factory().base72().unwrap().section();
    h.section(first.capability, first.region.token, 24)?;
    Ok(())
}

#[test]
fn actual_list_three_distinct_debug_allocations_and_four_live_locks_at_partial_cut() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let before = counts(&h);
    let final_cut = finish(&mut c, &mut h);
    let list = final_cut.list().unwrap();
    assert_eq!(
        h.memory.allocation_kind(list.token).unwrap(),
        Kind::SystemList
    );
    assert_eq!(list.data.len(), 24);
    assert_eq!(word(list, 0), list.token as u32);
    assert_eq!(word(list, 4), list.token as u32);
    assert_eq!(list.data[8..], final_cut.initial_list().unwrap().data[8..]);
    assert_eq!(h.memory.allocation_bytes(list.token).unwrap(), list.data);
    assert_eq!(counts(&h).0, before.0 + 24 + 3 * 32);
    assert_eq!(counts(&h).1, before.1 + 4);
    assert_eq!(counts(&h).2, before.2 + 4); // one binding, three section roles
    let sections = final_cut.sections();
    let first = final_cut.request().factory().base72().unwrap().section();
    let mut all = vec![first];
    all.extend(sections.iter().flatten());
    assert_eq!(all.len(), 4);
    for (at, section) in all.iter().enumerate() {
        let debug = section.debug_info.as_ref().unwrap();
        assert_eq!(debug.data.len(), 32);
        assert_eq!(
            h.memory.allocation_kind(debug.token).unwrap(),
            Kind::SystemSectionDebug
        );
        assert_eq!(h.memory.allocation_bytes(debug.token).unwrap(), debug.data);
        assert_eq!(word(debug, 4), section.region.token as u32);
        let lock = h
            .section(section.capability, section.region.token, 24)
            .unwrap();
        let guard = lock.lock().unwrap();
        assert!(
            h.section(section.capability, section.region.token, 24)
                .unwrap()
                .try_lock()
                .is_err()
        );
        for other in &all[..at] {
            assert_ne!(section.capability, other.capability);
            assert_ne!(debug.token, other.debug_info.as_ref().unwrap().token);
            assert!(!Arc::ptr_eq(
                lock,
                h.section(other.capability, other.region.token, 24).unwrap()
            ));
            assert!(
                h.section(other.capability, other.region.token, 24)
                    .unwrap()
                    .try_lock()
                    .is_ok()
            );
        }
        drop(guard);
    }
    lookups(&h, &final_cut).unwrap();
    assert!(h.memory.release_owner(list.token).is_err());
}

#[test]
fn resource_cut_preserves_suffix_and_normalizes_actual_section_getter_without_resource_return() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let initial = c.request().factory().parent().clone();
    let cp = finish(&mut c, &mut h);
    assert_eq!(word(cp.parent(), 128), 0x673884);
    assert_eq!(word(cp.parent(), 168), 0x676a00);
    assert_eq!(word(cp.parent(), 212), 0x673adc);
    assert_eq!(word(cp.parent(), 216), 0x673af8);
    assert_eq!(
        word(cp.parent(), 256),
        u32::from(word(cp.parent(), 248) != 0)
    );
    assert_eq!(cp.parent().data[260..264], initial.data[260..264]);
    assert_eq!(cp.parent().data[272..], initial.data[272..]);
    let arg = (cp.request().frames().resource_argument - cp.caller_frame().token) as usize;
    assert_eq!(word(cp.parent(), 264), word(cp.caller_frame(), arg));
    assert_eq!(word(cp.parent(), 268), 0);
    assert_eq!(cp.writes().last().unwrap().instruction, 0x5d9336);
    for forbidden in [
        0x5d9339, 0x5d9341, 0x5d9346, 0x6017f6, 0x6017fc, 0x5d5d34, 0x5d5d3c,
    ] {
        assert!(
            cp.writes()
                .iter()
                .all(|write| write.instruction != forbidden)
        );
    }
    assert!(matches!(
        c.pending_call(),
        Some(collection::Call::Resource {
            instruction: 0x5d9339,
            import_slot: 0x666238,
            arguments: [0, 0, 0, 0]
        })
    ));
    assert_eq!(
        cp.caller_frame().data[600..],
        cp.request().factory().caller().data[600..]
    );
    assert!(cp.caller_frame().data[600..].iter().any(|b| *b != 0));
    let before = counts(&h);
    assert!(step(&mut c, &mut h).is_err());
    assert_eq!(counts(&h), before);
    assert_eq!(
        h.memory.allocation_bytes(cp.parent().token).unwrap(),
        cp.parent().data
    );
}

#[test]
fn foreign_first_heap_with_identical_images_and_addresses_fails_before_collection_effect() {
    let (i, d) = fixture();
    let mut a = services(&i, &d);
    let mut b = services(&i, &d);
    let mut ca = cursor(&mut a);
    let mut cb = cursor(&mut b);
    let pa = capture(&mut ca, &mut a, 0);
    // b has its actual factory but NO collection binding at first foreign call.
    assert_eq!(
        ca.request().factory().parent(),
        cb.request().factory().parent()
    );
    assert_eq!(
        ca.request().factory().caller(),
        cb.request().factory().caller()
    );
    assert_ne!(a.memory.system_origin(), b.memory.system_origin());
    let before = counts(&b);
    assert!(synchronize(&mut b, cut(&pa)).is_err());
    assert!(issue(&mut b, &pa).is_err());
    assert_eq!(counts(&b), before);
    let pb = capture(&mut cb, &mut b, 0);
    assert_eq!(cut(&pa).parent(), cut(&pb).parent());
    assert_eq!(cut(&pa).caller_frame(), cut(&pb).caller_frame());
    let before = counts(&b);
    assert!(issue(&mut b, &pa).is_err());
    assert_eq!(counts(&b), before);
    finish(&mut cb, &mut b);
}

#[test]
fn identical_effect_retries_keep_actual_list_debug_and_lock_without_source_rewind() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let mut calls = Vec::new();
    for stage in 0..4 {
        let p = capture(&mut c, &mut h, stage);
        issue(&mut h, &p).unwrap();
        let after = counts(&h);
        let current = current_images(&h, c.request()).unwrap().0.clone();
        synchronize(&mut h, cut(&p)).unwrap(); // exact unconsumed-reply seam
        assert_eq!(
            h.memory.allocation_bytes(current.token).unwrap(),
            current.data
        );
        issue(&mut h, &p).unwrap();
        assert_eq!(counts(&h), after);
        calls.push(p);
    }
    let cp = finish(&mut c, &mut h);
    let locks: Vec<_> = cp
        .sections()
        .iter()
        .flatten()
        .map(|s| h.section(s.capability, s.region.token, 24).unwrap().clone())
        .collect();
    let before = counts(&h);
    for p in calls {
        issue(&mut h, &p).unwrap();
    }
    assert_eq!(counts(&h), before);
    assert_eq!(
        h.memory.allocation_bytes(cp.parent().token).unwrap(),
        cp.parent().data
    );
    assert_eq!(
        h.memory.allocation_bytes(cp.caller_frame().token).unwrap(),
        cp.caller_frame().data
    );
    for (lock, section) in locks.iter().zip(cp.sections().iter().flatten()) {
        assert!(Arc::ptr_eq(
            lock,
            h.section(section.capability, section.region.token, 24)
                .unwrap()
        ));
    }
}

#[test]
fn all_four_effects_survive_lost_reply_then_retry_without_duplicate_allocations() {
    for stage in 0..4 {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let p = capture(&mut c, &mut h, stage);
        let before = counts(&h);
        let request = c.request().clone();
        let mut adapter = Adapter {
            h: &mut h,
            request,
            hold: None,
            pending: None,
            lose: Some(stage),
        };
        assert!(c.step(&mut adapter).is_err());
        drop(adapter);
        let after = counts(&h);
        assert_eq!(after.1, before.1 + 1);
        // Simulate capability exhaustion AFTER effect: retained replies still
        // work and cannot reserve/reissue another effect during recovery.
        let saved_next = h.next;
        h.next = CAP_START + MAX_CAPABILITIES;
        issue(&mut h, &p).unwrap();
        assert_eq!(h.memory.live_allocations(), after.1);
        assert_eq!(h.memory.live_bytes(), after.0);
        h.next = saved_next;
        let cp = finish(&mut c, &mut h);
        assert_eq!(cp.sections().iter().flatten().count(), 3);
        lookups(&h, &cp).unwrap();
    }
}

#[test]
fn allocator_quota_before_each_effect_preserves_pending_call_and_allows_retry() {
    for stage in 0..4 {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let p = capture(&mut c, &mut h, stage);
        let bytes = if stage == 1 { 24 } else { 32 };
        let scratch = h
            .memory
            .allocate_owner((8192 - h.memory.live_bytes() - bytes + 1) as u32)
            .unwrap();
        let before = counts(&h);
        let parent = h
            .memory
            .allocation_bytes(cut(&p).parent().token)
            .unwrap()
            .to_vec();
        let caller = h
            .memory
            .allocation_bytes(cut(&p).caller_frame().token)
            .unwrap()
            .to_vec();
        assert!(matches!(issue(&mut h, &p), Err(Stop::Unavailable(_))));
        assert_eq!(counts(&h), before);
        assert_eq!(
            h.memory.allocation_bytes(cut(&p).parent().token).unwrap(),
            parent
        );
        assert_eq!(
            h.memory
                .allocation_bytes(cut(&p).caller_frame().token)
                .unwrap(),
            caller
        );
        h.memory.release_owner(scratch.token).unwrap();
        issue(&mut h, &p).unwrap();
        let issued = counts(&h);
        issue(&mut h, &p).unwrap();
        assert_eq!(counts(&h), issued);
        finish(&mut c, &mut h);
    }
}

#[test]
fn owning_binding_and_each_section_reserve_capability_before_allocation() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let saved = h.next;
    h.next = CAP_START + MAX_CAPABILITIES;
    let before = counts(&h);
    assert!(step_until_error(&mut c, &mut h));
    assert_eq!(counts(&h), before);
    h.next = saved;
    for stage in [0, 2, 3] {
        let p = capture(&mut c, &mut h, stage);
        let saved = h.next;
        h.next = CAP_START + MAX_CAPABILITIES;
        let before = counts(&h);
        assert!(matches!(issue(&mut h, &p), Err(Stop::Unavailable(_))));
        assert_eq!(counts(&h), before);
        h.next = saved;
        issue(&mut h, &p).unwrap();
    }
    finish(&mut c, &mut h);
}
fn step_until_error(c: &mut collection::Construction, h: &mut BootstrapServices<'_>) -> bool {
    for _ in 0..1024 {
        if step(c, h).is_err() {
            return true;
        }
    }
    false
}

#[test]
fn historical_checkpoint_cannot_reset_newer_images_even_when_its_call_retry_is_valid() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let p = capture(&mut c, &mut h, 0);
    let old = cut(&p).clone();
    let cp = finish(&mut c, &mut h);
    assert!(old.source_ordinal() < cp.source_ordinal());
    let before = counts(&h);
    assert!(synchronize(&mut h, &old).is_err());
    issue(&mut h, &p).unwrap();
    synchronize(&mut h, &cp).unwrap();
    synchronize(&mut h, &cp).unwrap();
    assert_eq!(counts(&h), before);
    assert_eq!(
        h.memory.allocation_bytes(cp.parent().token).unwrap(),
        cp.parent().data
    );
    assert_eq!(
        h.memory.allocation_bytes(cp.caller_frame().token).unwrap(),
        cp.caller_frame().data
    );
    lookups(&h, &cp).unwrap();
}

fn retained_owners(cp: &Checkpoint) -> Vec<(Owner, Kind)> {
    let factory = cp.request().factory();
    let mut owners = vec![
        (cp.parent().clone(), Kind::Owner),
        (cp.caller_frame().clone(), Kind::Owner),
        (cp.list().unwrap().clone(), Kind::SystemList),
    ];
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        owners.push((factory.tree(site).unwrap().clone(), Kind::SystemTree));
    }
    owners.push((
        factory
            .base72()
            .unwrap()
            .section()
            .debug_info
            .as_ref()
            .unwrap()
            .clone(),
        Kind::SystemSectionDebug,
    ));
    for section in cp.sections().iter().flatten() {
        owners.push((
            section.debug_info.as_ref().unwrap().clone(),
            Kind::SystemSectionDebug,
        ));
    }
    owners
}

#[test]
fn any_current_full_image_or_debug_tamper_is_rejected_without_adoption() {
    for role in 0..9 {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let p = capture(&mut c, &mut h, 0);
        let cp = finish(&mut c, &mut h);
        let mut owner = retained_owners(&cp)[role].0.clone();
        let at = if role == 0 {
            900
        } else if role == 2 {
            23
        } else {
            0
        };
        owner.data[at] ^= 1;
        h.memory.synchronize_owner(&owner).unwrap();
        let before = counts(&h);
        assert!(lookups(&h, &cp).is_err());
        assert!(synchronize(&mut h, &cp).is_err());
        assert!(issue(&mut h, &p).is_err());
        assert_eq!(counts(&h), before);
        assert_eq!(h.memory.allocation_bytes(owner.token).unwrap(), owner.data);
    }
}

#[test]
fn released_parent_caller_list_tree_or_debug_invalidates_every_collection_lookup() {
    for role in 0..9 {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let cp = finish(&mut c, &mut h);
        let (owner, kind) = &retained_owners(&cp)[role];
        h.memory.release(owner.token, *kind).unwrap();
        let before = counts(&h);
        assert!(lookups(&h, &cp).is_err());
        assert!(synchronize(&mut h, &cp).is_err());
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn swapped_collection_section_capabilities_or_sealed_call_roles_are_rejected() {
    for swap_calls_only in [false, true] {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let cp = finish(&mut c, &mut h);
        let a = cp.sections()[0].as_ref().unwrap().capability as u32;
        let b = cp.sections()[1].as_ref().unwrap().capability as u32;
        let mut ca = h.capabilities.remove(&a).unwrap();
        let mut cb = h.capabilities.remove(&b).unwrap();
        if swap_calls_only {
            let Capability::SystemCollectionSection(sa) = &mut ca else {
                panic!("actual section role")
            };
            let Capability::SystemCollectionSection(sb) = &mut cb else {
                panic!("actual section role")
            };
            std::mem::swap(&mut sa.call, &mut sb.call);
            h.capabilities.insert(a, ca);
            h.capabilities.insert(b, cb);
        } else {
            h.capabilities.insert(a, cb);
            h.capabilities.insert(b, ca);
        }
        let before = counts(&h);
        assert!(lookups(&h, &cp).is_err());
        assert!(synchronize(&mut h, &cp).is_err());
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn lost_replaced_or_wrong_role_capability_cannot_be_recreated_by_replay() {
    for role in 0..4 {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let p = capture(&mut c, &mut h, 0);
        let cp = finish(&mut c, &mut h);
        let token = if role == 0 {
            key(&h, c.request()).unwrap().unwrap()
        } else {
            cp.sections()[role - 1].as_ref().unwrap().capability as u32
        };
        h.capabilities.remove(&token).unwrap();
        let before = counts(&h);
        assert!(lookups(&h, &cp).is_err());
        assert!(issue(&mut h, &p).is_err());
        assert!(synchronize(&mut h, &cp).is_err());
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn equal_image_whole_heap_swap_and_foreign_call_replay_are_rejected() {
    let (i, d) = fixture();
    let mut a = services(&i, &d);
    let mut b = services(&i, &d);
    let mut ca = cursor(&mut a);
    let mut cb = cursor(&mut b);
    let p = capture(&mut ca, &mut a, 0);
    let cpa = finish(&mut ca, &mut a);
    let cpb = finish(&mut cb, &mut b);
    assert_eq!(cpa.parent(), cpb.parent());
    assert_eq!(cpa.caller_frame(), cpb.caller_frame());
    let before = counts(&b);
    assert!(issue(&mut b, &p).is_err());
    assert!(synchronize(&mut b, &cpa).is_err());
    assert_eq!(counts(&b), before);
    std::mem::swap(&mut a.memory, &mut b.memory);
    assert!(lookups(&a, &cpa).is_err());
    assert!(lookups(&b, &cpb).is_err());
    assert!(issue(&mut a, &p).is_err());
}

#[test]
fn generic_section_replay_shifted_regions_cross_role_lookups_and_native_width_fail() {
    use sims_sim::native_application::Host as _;
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let p = capture(&mut c, &mut h, 0);
    let Pending::Section(call) = &p else {
        panic!("collection section call")
    };
    let request = call.section_request();
    let before = counts(&h);
    assert!(h.application_host().initialize_section(&request).is_err());
    for delta in [1, 4, 44, 88] {
        let mut shifted = request.clone();
        shifted.region.token += delta;
        assert!(h.application_host().initialize_section(&shifted).is_err());
    }
    assert_eq!(counts(&h), before);
    let cp = finish(&mut c, &mut h);
    for section in cp.sections().iter().flatten() {
        for (token, address, bytes) in [
            (section.capability, section.region.token + 1, 24),
            (section.capability, section.region.token, 23),
            (section.capability + (1_u64 << 32), section.region.token, 24),
            (CAP_START as u64, section.region.token, 24),
        ] {
            assert!(h.section(token, address, bytes).is_err());
            assert!(h.section_debug_info(token, address, bytes).is_err());
        }
        for other in cp.sections().iter().flatten() {
            if section.capability != other.capability {
                assert!(
                    h.section(section.capability, other.region.token, 24)
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn exact_call_identity_differs_for_each_section_even_with_shared_import_and_parent() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let mut calls = Vec::new();
    for stage in [0, 2, 3] {
        let p = capture(&mut c, &mut h, stage);
        let Pending::Section(call) = p else {
            panic!("source section")
        };
        assert!(call.same_call(&call.clone()));
        for previous in &calls {
            assert!(!call.same_call(previous));
        }
        let request = call.section_request();
        assert_eq!(
            (request.instruction, request.import_slot),
            (0x5d91bc, 0x666114)
        );
        let returned = initialize_section(&mut h, &call).unwrap();
        returned.validate(&request).unwrap();
        refresh_first(&mut h, c.request());
        calls.push(call);
    }
    let cp = finish(&mut c, &mut h);
    let before = counts(&h);
    for (at, call) in calls.iter().enumerate() {
        assert_eq!(
            initialize_section(&mut h, call).unwrap(),
            *cp.sections()[at].as_ref().unwrap()
        );
    }
    assert_eq!(counts(&h), before);
}

#[test]
fn changed_live_preimages_at_each_pending_effect_fail_before_any_new_allocation() {
    for stage in 0..4 {
        for role in 0..4 {
            let (i, d) = fixture();
            let mut h = services(&i, &d);
            let mut c = cursor(&mut h);
            let p = capture(&mut c, &mut h, stage);
            let mut owner = match role {
                0 => cut(&p).parent().clone(),
                1 => cut(&p).caller_frame().clone(),
                2 => c
                    .request()
                    .factory()
                    .tree(TreeSite::Derived)
                    .unwrap()
                    .clone(),
                _ => c
                    .request()
                    .factory()
                    .base72()
                    .unwrap()
                    .section()
                    .debug_info
                    .as_ref()
                    .unwrap()
                    .clone(),
            };
            let offset = if role == 0 { 900 } else { 0 };
            owner.data[offset] ^= 1;
            h.memory.synchronize_owner(&owner).unwrap();
            let before = counts(&h);
            assert!(issue(&mut h, &p).is_err());
            assert_eq!(counts(&h), before);
            assert_eq!(h.memory.allocation_bytes(owner.token).unwrap(), owner.data);
        }
    }
}

#[test]
fn relocated_owning_capability_or_replaced_rust_lock_fails_lifetime_checks() {
    for replace_lock in [false, true] {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let cp = finish(&mut c, &mut h);
        if replace_lock {
            let token = cp.sections()[0].as_ref().unwrap().capability as u32;
            let Some(Capability::SystemCollectionSection(section)) = h.capabilities.get_mut(&token)
            else {
                panic!("actual collection section")
            };
            section.lock = Arc::new(Mutex::new(()));
        } else {
            let token = key(&h, c.request()).unwrap().unwrap();
            let cap = h.capabilities.remove(&token).unwrap();
            assert!(
                h.capabilities
                    .insert(CAP_START + MAX_CAPABILITIES - 1, cap)
                    .is_none()
            );
        }
        let before = counts(&h);
        assert!(lookups(&h, &cp).is_err());
        assert!(synchronize(&mut h, &cp).is_err());
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn lost_issuing_factory_capability_invalidates_bound_collection_resources() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let p = capture(&mut c, &mut h, 0);
    let cp = finish(&mut c, &mut h);
    let factory = *h
        .capabilities
        .iter()
        .find(|(_, cap)| matches!(cap, Capability::SystemFactory(_)))
        .unwrap()
        .0;
    h.capabilities.remove(&factory).unwrap();
    let before = counts(&h);
    assert!(lookups(&h, &cp).is_err());
    assert!(synchronize(&mut h, &cp).is_err());
    assert!(issue(&mut h, &p).is_err());
    assert_eq!(counts(&h), before);
}

#[test]
fn corrupted_retained_section_reply_is_rejected_without_a_replacement_effect() {
    for stage in [0, 2, 3] {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = cursor(&mut h);
        let p = capture(&mut c, &mut h, stage);
        issue(&mut h, &p).unwrap();
        let Pending::Section(call) = &p else {
            panic!("typed source section")
        };
        let at = site_index(call.site());
        let root = key(&h, c.request()).unwrap().unwrap();
        let cap = binding(&h, root).unwrap().section_caps[at].unwrap();
        let Some(Capability::SystemCollectionSection(section)) = h.capabilities.get_mut(&cap)
        else {
            panic!("actual section reply retained before handoff")
        };
        section.returned.region.data[23] ^= 1;
        let before = counts(&h);
        assert!(issue(&mut h, &p).is_err());
        assert!(step(&mut c, &mut h).is_err());
        assert_eq!(counts(&h), before);
        assert!(h.capabilities.contains_key(&cap));
    }
}

#[test]
fn parent_refresh_is_required_and_existing_first_base72_lifetime_check_stays_strict() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let p = capture(&mut c, &mut h, 0);
    let Pending::Section(call) = &p else {
        panic!("first collection section")
    };
    let returned = initialize_section(&mut h, call).unwrap(); // deliberately omit parent handoff
    let first = c.request().factory().base72().unwrap().section();
    assert!(h.section(first.capability, first.region.token, 24).is_err());
    assert!(
        h.section(returned.capability, returned.region.token, 24)
            .is_err()
    );
    let before = counts(&h);
    refresh_first(&mut h, c.request());
    assert!(h.section(first.capability, first.region.token, 24).is_ok());
    assert!(
        h.section(returned.capability, returned.region.token, 24)
            .is_ok()
    );
    assert_eq!(counts(&h), before);
    finish(&mut c, &mut h);
}

fn finish_parent(c: &mut collection::Construction, h: &mut BootstrapServices<'_>) {
    for _ in 0..256 {
        let request = c.request().clone();
        if c.step(&mut h.system_collection_host(&request)).unwrap()
            == Some(collection::Cut::BeforeResource)
        {
            return;
        }
    }
    panic!("parent adapter did not reach the resource boundary");
}

#[test]
fn parent_adapter_retains_factory_and_all_live_sections_without_publishing_old_images() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    finish_parent(&mut c, &mut h);
    let cut = c.checkpoint().unwrap();
    let parent = h
        .memory
        .allocation_bytes(cut.parent().token)
        .unwrap()
        .to_vec();
    let before = counts(&h);
    let factory_request = c.request().factory().request().clone();
    assert!(
        factory::Host::allocate_provider(
            &mut h.system_provider_host(&factory_request),
            &factory_request
        )
        .is_ok()
    );
    assert!(system_factory::synchronize(&mut h, c.request().factory()).is_err());
    assert_eq!(
        h.memory.allocation_bytes(cut.parent().token).unwrap(),
        parent
    );
    assert_eq!(counts(&h), before);
    for section in cut.sections().iter().flatten() {
        assert!(
            h.section(section.capability, section.region.token, 24)
                .is_ok()
        );
        assert_eq!(
            h.section_debug_info(section.capability, section.region.token, 24)
                .unwrap(),
            section.debug_info.as_ref()
        );
    }
    let first = c.request().factory().base72().unwrap().section();
    assert!(h.section(first.capability, first.region.token, 24).is_ok());
}

#[test]
fn parent_adapter_refreshes_shared_images_after_quota_error_then_retries_same_call() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let scratch = h
        .memory
        .allocate_owner((8192 - h.memory.live_bytes() - 31) as u32)
        .unwrap();
    let mut stopped = false;
    for _ in 0..64 {
        let request = c.request().clone();
        match c.step(&mut h.system_collection_host(&request)) {
            Ok(_) => (),
            Err(Stop::Unavailable(_)) => {
                stopped = true;
                break;
            }
            Err(stop) => panic!("unexpected parent stop {stop:?}"),
        }
    }
    assert!(stopped);
    let first = c.request().factory().base72().unwrap().section().clone();
    assert!(
        h.section(first.capability, first.region.token, 24).is_ok(),
        "Err path left first-child images stale"
    );
    h.memory.release_owner(scratch.token).unwrap();
    finish_parent(&mut c, &mut h);
    assert!(h.section(first.capability, first.region.token, 24).is_ok());
}

#[test]
fn parent_preflight_does_not_repair_a_tampered_first_section_record() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    finish_parent(&mut c, &mut h);
    let first_token = c.request().factory().base72().unwrap().section().capability as u32;
    let Some(Capability::SystemBase72(first)) = h.capabilities.get_mut(&first_token) else {
        panic!("first section");
    };
    first.parent_image.data[0] ^= 1;
    let altered = first.parent_image.clone();
    let before = counts(&h);
    let request = c.request().clone();
    let cut = c.checkpoint().unwrap();
    for _ in 0..2 {
        assert!(
            collection::Host::synchronize_checkpoint(&mut h.system_collection_host(&request), &cut)
                .is_err()
        );
        let Some(Capability::SystemBase72(first)) = h.capabilities.get(&first_token) else {
            panic!("first section");
        };
        assert_eq!(first.parent_image, altered);
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn parent_host_rejects_another_source_request_before_any_shared_image_handoff() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let other = collection::Construction::new(collection::Incoming {
        factory: c.request().factory().clone(),
        frames: c.request().frames(),
    })
    .unwrap();
    finish_parent(&mut c, &mut h);
    let request = c.request().clone();
    let before = counts(&h);
    assert!(
        collection::Host::synchronize_checkpoint(
            &mut h.system_collection_host(&request),
            &other.checkpoint().unwrap()
        )
        .is_err()
    );
    assert_eq!(counts(&h), before);
}

#[test]
fn parent_takeover_record_rejects_missing_collection_owner_and_old_factory_reset() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    finish_parent(&mut c, &mut h);
    let root = h.capabilities.iter().find_map(|(token, cap)| {
        matches!(cap, Capability::SystemCollection(binding) if binding.request().same_call(c.request())).then_some(*token)
    }).unwrap();
    h.capabilities.remove(&root).unwrap();
    let before = counts(&h);
    let parent = h
        .memory
        .allocation_bytes(c.request().factory().parent().token)
        .unwrap()
        .to_vec();
    let factory_request = c.request().factory().request().clone();
    assert!(
        factory::Host::allocate_provider(
            &mut h.system_provider_host(&factory_request),
            &factory_request
        )
        .is_err()
    );
    assert!(system_factory::synchronize(&mut h, c.request().factory()).is_err());
    assert_eq!(counts(&h), before);
    assert_eq!(
        h.memory
            .allocation_bytes(c.request().factory().parent().token)
            .unwrap(),
        parent
    );
}

#[test]
fn parent_host_rejects_the_first_foreign_heap_even_with_equal_factory_images() {
    let (i, d) = fixture();
    let mut issuing = services(&i, &d);
    let mut c = cursor(&mut issuing);
    let mut foreign = services(&i, &d);
    let other = factory_frontier(&mut foreign);
    assert_eq!(other.parent(), c.request().factory().parent());
    assert_eq!(other.caller(), c.request().factory().caller());
    let before = counts(&foreign);
    let mut rejected = false;
    for _ in 0..32 {
        let request = c.request().clone();
        if c.step(&mut foreign.system_collection_host(&request))
            .is_err()
        {
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    assert_eq!(counts(&foreign), before);
}
