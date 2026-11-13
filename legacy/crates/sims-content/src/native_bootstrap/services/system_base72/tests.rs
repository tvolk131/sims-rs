//! Parent replaces the worker's pre-origin API tests with actual factory-origin
//! controls. The18 original UNRUN controls remain in worker-b-delivery-01.
//! These use real logical Memory and BootstrapServices; no native/IPC claim.
use super::*;
use sims_sim::native_application::InitializedSection;
use sims_sim::native_system::{
    base72,
    factory::{
        self, Base72Call, Checkpoint, Construction, Cut, Frames, Incoming, TreeRequest, TreeSite,
    },
};

fn fixture() -> (Index, Directories) {
    (
        Index {
            root: "system-source-tests".into(),
            resources: vec![],
            archives: 0,
            diagnostics: vec![],
        },
        Directories::default(),
    )
}
fn host<'a>(index: &'a Index, dirs: &'a Directories, limit: usize) -> BootstrapServices<'a> {
    BootstrapServices::new(Memory::new(limit).unwrap(), index, dirs, LocalePolicy::C)
}
fn cursor(host: &mut BootstrapServices<'_>) -> Construction {
    let mut caller = host.memory.allocate_owner(384).unwrap();
    for (at, byte) in caller.data.iter_mut().enumerate() {
        *byte = (at as u8).wrapping_mul(19).wrapping_add(7);
    }
    host.memory.synchronize_owner(&caller).unwrap();
    let b = caller.token;
    Construction::new(Incoming {
        origin: host.memory.system_origin(),
        caller,
        frames: Frames {
            factory: b + 32,
            derived: b + 80,
            base: b + 128,
            refcount: b + 176,
            child: b + 224,
            base72: b + 288,
        },
    })
    .unwrap()
}
fn step(c: &mut Construction, h: &mut BootstrapServices<'_>) -> Result<Option<Cut>, Stop> {
    let request = c.request().clone();
    c.step(&mut h.system_provider_host(&request))
}
fn finish(c: &mut Construction, h: &mut BootstrapServices<'_>) {
    for _ in 0..512 {
        if step(c, h).unwrap() == Some(Cut::FirstDerivedChildReturned) {
            return;
        }
    }
    panic!("factory did not reach its source child return");
}
fn counts(h: &BootstrapServices<'_>) -> (usize, usize, usize, u32) {
    (
        h.memory.live_bytes(),
        h.memory.live_allocations(),
        h.live_capabilities(),
        h.next,
    )
}
struct Hold<'s, 'a> {
    inner: SystemProviderHost<'s, 'a>,
}
impl factory::Host for Hold<'_, '_> {
    fn allocate_provider(&mut self, r: &factory::Request) -> Result<Allocated, Stop> {
        factory::Host::allocate_provider(&mut self.inner, r)
    }
    fn allocate_tree(&mut self, r: &TreeRequest) -> Result<Allocated, Stop> {
        factory::Host::allocate_tree(&mut self.inner, r)
    }
    fn prepare_base72(&mut self, r: &Checkpoint) -> Result<(), Stop> {
        factory::Host::prepare_base72(&mut self.inner, r)
    }
    fn initialize_base72(&mut self, _: &Base72Call) -> Result<InitializedSection, Stop> {
        Err(Stop::Unavailable(
            "test holds actual source import before resource effect".into(),
        ))
    }
    fn acknowledge_base72(&mut self, c: &Base72Call, p: &base72::Prefix) -> Result<(), Stop> {
        factory::Host::acknowledge_base72(&mut self.inner, c, p)
    }
    fn synchronize_checkpoint(&mut self, c: &Checkpoint) -> Result<(), Stop> {
        factory::Host::synchronize_checkpoint(&mut self.inner, c)
    }
}
fn pending(c: &mut Construction, h: &mut BootstrapServices<'_>) -> Base72Call {
    for _ in 0..512 {
        let request = c.request().clone();
        let result = c.step(&mut Hold {
            inner: h.system_provider_host(&request),
        });
        if result.is_err() {
            return c
                .base72_call()
                .expect("held after actual source seals its child call")
                .clone();
        }
    }
    panic!("source import not reached");
}
fn prefix<'b>(c: &'b Construction) -> &'b base72::Prefix {
    c.base72_prefix().unwrap()
}
fn check_lookup<'b>(
    h: &'b BootstrapServices<'_>,
    c: &Construction,
) -> Result<&'b Arc<Mutex<()>>, Stop> {
    let section = prefix(c).section();
    h.section(section.capability, section.region.token, 24)
}

#[test]
fn system_base72_factory_origin_actual_resources_and_later_source_stores_remain_live() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    finish(&mut c, &mut h);
    let cp = c.checkpoint().unwrap();
    cp.validate().unwrap();
    let p = prefix(&c);
    assert_eq!(
        h.memory.allocation_bytes(cp.parent().token).unwrap(),
        cp.parent().data
    );
    assert_eq!(
        h.memory.allocation_bytes(cp.caller().token).unwrap(),
        cp.caller().data
    );
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        let tree = cp.tree(site).unwrap();
        assert_eq!(h.memory.allocation_bytes(tree.token).unwrap(), tree.data);
        assert_eq!(
            h.memory.allocation_kind(tree.token).unwrap(),
            Kind::SystemTree
        );
    }
    let section = p.section();
    let debug = section.debug_info.as_ref().unwrap();
    assert_eq!(
        h.section_debug_info(section.capability, section.region.token, 24)
            .unwrap(),
        Some(debug)
    );
    let lock = check_lookup(&h, &c).unwrap().clone();
    assert!(Arc::ptr_eq(&lock, check_lookup(&h, &c).unwrap()));
    let guard = lock.lock().unwrap();
    assert!(check_lookup(&h, &c).unwrap().try_lock().is_err());
    drop(guard);
    assert!(check_lookup(&h, &c).unwrap().try_lock().is_ok());
    assert_eq!((h.memory.live_allocations(), h.live_capabilities()), (5, 2));
    assert!(h.memory.release_owner(debug.token).is_err());
    assert!(
        h.memory
            .release_owner(cp.tree(TreeSite::Refcount).unwrap().token)
            .is_err()
    );
    let before = counts(&h);
    let reply = system_base72::initialize(&mut h, p.request()).unwrap();
    assert_eq!(reply, *p.section());
    assert_eq!(counts(&h), before);
    assert_eq!(
        h.memory.allocation_bytes(cp.parent().token).unwrap(),
        cp.parent().data,
        "retry reset later source vptrs"
    );
}

#[test]
fn system_base72_foreign_identical_initial_heap_requires_origin_provenance() {
    let (i, d) = fixture();
    let mut a = host(&i, &d, 8192);
    let mut b = host(&i, &d, 8192);
    let mut ca = cursor(&mut a);
    let mut cb = cursor(&mut b);
    let call_a = pending(&mut ca, &mut a);
    let call_b = pending(&mut cb, &mut b);
    assert_eq!(call_a.checkpoint().parent(), call_b.checkpoint().parent());
    assert_eq!(call_a.checkpoint().caller(), call_b.checkpoint().caller());
    assert_ne!(a.memory.system_origin(), b.memory.system_origin());
    let before = counts(&b);
    assert!(system_factory::initialize_base72(&mut b, &call_a).is_err());
    assert!(system_base72::initialize(&mut b, call_a.request()).is_err());
    let error = system_factory::allocate(&mut b, ca.request()).unwrap_err();
    assert!(format!("{error:?}").contains("origin"));
    assert_eq!(counts(&b), before);
    finish(&mut cb, &mut b);
    assert!(check_lookup(&b, &cb).is_ok());
}

#[test]
fn system_base72_foreign_factory_first_allocation_is_rejected_before_effects() {
    let (i, d) = fixture();
    let mut a = host(&i, &d, 8192);
    let mut b = host(&i, &d, 8192);
    let ca = cursor(&mut a);
    let _cb = cursor(&mut b);
    assert_eq!(
        a.memory
            .allocation_bytes(ca.request().incoming().caller.token)
            .unwrap(),
        b.memory
            .allocation_bytes(ca.request().incoming().caller.token)
            .unwrap()
    );
    let before = counts(&b);
    assert!(system_factory::allocate(&mut b, ca.request()).is_err());
    assert_eq!(counts(&b), before);
}

#[test]
fn system_base72_source_call_and_ack_retry_retain_one_actual_allocation_and_lock() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    let call = pending(&mut c, &mut h);
    let before = counts(&h);
    let first = system_factory::initialize_base72(&mut h, &call).unwrap();
    let issued = counts(&h);
    assert_eq!(issued.0, before.0 + 32);
    assert_eq!(issued.1, before.1 + 1);
    assert_eq!(issued.2, before.2 + 1);
    assert_eq!(
        system_factory::initialize_base72(&mut h, &call).unwrap(),
        first
    );
    assert_eq!(counts(&h), issued);
    for _ in 0..512 {
        if step(&mut c, &mut h).unwrap() == Some(Cut::Base72Returned) {
            break;
        }
    }
    let lock = check_lookup(&h, &c).unwrap().clone();
    let before = counts(&h);
    system_factory::acknowledge(&mut h, &call, prefix(&c)).unwrap();
    system_factory::acknowledge(&mut h, &call, prefix(&c)).unwrap();
    assert_eq!(counts(&h), before);
    assert!(Arc::ptr_eq(&lock, check_lookup(&h, &c).unwrap()));
}

#[test]
fn system_base72_capability_exhaustion_precedes_factory_and_section_effects() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let c = cursor(&mut h);
    h.next = CAP_START + MAX_CAPABILITIES;
    let before = counts(&h);
    assert!(matches!(
        system_factory::allocate(&mut h, c.request()),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(counts(&h), before);
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    let call = pending(&mut c, &mut h);
    let old = h.next;
    h.next = CAP_START + MAX_CAPABILITIES;
    let before = counts(&h);
    assert!(matches!(
        system_factory::initialize_base72(&mut h, &call),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(counts(&h), before);
    h.next = old;
    finish(&mut c, &mut h);
    assert!(check_lookup(&h, &c).is_ok());
}

#[test]
fn system_base72_allocator_quota_preserves_images_and_retry_uses_same_source_call() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 1792);
    let scratch = h.memory.allocate_owner(128).unwrap();
    let mut c = cursor(&mut h);
    let call = pending(&mut c, &mut h);
    let before = counts(&h);
    let parent = h
        .memory
        .allocation_bytes(call.request().initial_parent().token)
        .unwrap()
        .to_vec();
    assert!(matches!(
        system_factory::initialize_base72(&mut h, &call),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(counts(&h), before);
    assert_eq!(
        h.memory
            .allocation_bytes(call.request().initial_parent().token)
            .unwrap(),
        parent
    );
    h.memory.release_owner(scratch.token).unwrap();
    finish(&mut c, &mut h);
    assert!(check_lookup(&h, &c).is_ok());
}

#[test]
fn system_base72_full_parent_and_caller_preimage_tampering_precedes_debug_allocation() {
    for caller in [false, true] {
        let (i, d) = fixture();
        let mut h = host(&i, &d, 8192);
        let mut c = cursor(&mut h);
        let call = pending(&mut c, &mut h);
        let mut image = if caller {
            call.checkpoint().caller().clone()
        } else {
            call.checkpoint().parent().clone()
        };
        image.data[0] ^= 1;
        h.memory.synchronize_owner(&image).unwrap();
        let before = counts(&h);
        assert!(system_factory::initialize_base72(&mut h, &call).is_err());
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn system_base72_released_parent_caller_and_debug_invalidate_all_live_lookups() {
    for role in [0, 1, 2] {
        let (i, d) = fixture();
        let mut h = host(&i, &d, 8192);
        let mut c = cursor(&mut h);
        finish(&mut c, &mut h);
        let cp = c.checkpoint().unwrap();
        match role {
            0 => h.memory.release_owner(cp.parent().token).unwrap(),
            1 => h.memory.release_owner(cp.caller().token).unwrap(),
            _ => h
                .memory
                .release(
                    prefix(&c).section().debug_info.as_ref().unwrap().token,
                    Kind::SystemSectionDebug,
                )
                .unwrap(),
        };
        assert!(check_lookup(&h, &c).is_err());
        assert!(system_base72::initialize(&mut h, prefix(&c).request()).is_err());
    }
}

#[test]
fn system_base72_byte_identical_whole_heap_swap_and_foreign_prefix_are_rejected() {
    let (i, d) = fixture();
    let mut a = host(&i, &d, 8192);
    let mut b = host(&i, &d, 8192);
    let mut ca = cursor(&mut a);
    let mut cb = cursor(&mut b);
    finish(&mut ca, &mut a);
    finish(&mut cb, &mut b);
    assert_eq!(
        ca.checkpoint().unwrap().parent(),
        cb.checkpoint().unwrap().parent()
    );
    let before = counts(&b);
    assert!(system_base72::acknowledge(&mut b, prefix(&ca)).is_err());
    assert_eq!(counts(&b), before);
    std::mem::swap(&mut a.memory, &mut b.memory);
    assert!(check_lookup(&a, &ca).is_err());
    assert!(system_base72::initialize(&mut a, prefix(&ca).request()).is_err());
}

#[test]
fn system_base72_tampering_in_unwritten_padding_or_debug_after_issue_is_not_adopted() {
    for role in [0, 1, 2] {
        let (i, d) = fixture();
        let mut h = host(&i, &d, 8192);
        let mut c = cursor(&mut h);
        finish(&mut c, &mut h);
        let cp = c.checkpoint().unwrap();
        let mut image = match role {
            0 => cp.parent().clone(),
            1 => cp.caller().clone(),
            _ => prefix(&c).section().debug_info.as_ref().unwrap().clone(),
        };
        let offset = if role == 0 { 900 } else { 0 };
        image.data[offset] ^= 1;
        h.memory.synchronize_owner(&image).unwrap();
        let before = counts(&h);
        assert!(check_lookup(&h, &c).is_err());
        assert!(system_base72::acknowledge(&mut h, prefix(&c)).is_err());
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn system_base72_wrong_section_kind_address_extent_and_native_width_stay_rejected() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    finish(&mut c, &mut h);
    let p = prefix(&c);
    let section = p.section();
    let before = counts(&h);
    for (token, address, bytes) in [
        (section.capability, section.region.token + 1, 24),
        (section.capability, section.region.token, 23),
        (section.capability + (1_u64 << 32), section.region.token, 24),
        (CAP_START as u64, section.region.token, 24),
    ] {
        assert!(h.section(token, address, bytes).is_err());
        assert!(h.section_debug_info(token, address, bytes).is_err());
    }
    assert_eq!(counts(&h), before);
    use sims_sim::native_application::Host as _;
    assert!(
        h.application_host()
            .initialize_section(&p.request().section_request())
            .is_err()
    );
    assert_eq!(counts(&h), before);
}

#[test]
fn system_base72_distinct_factory_invocation_cannot_reuse_retained_caller() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    let _ = pending(&mut c, &mut h);
    let mut incoming = c.request().incoming().clone();
    incoming.caller.data = h
        .memory
        .allocation_bytes(incoming.caller.token)
        .unwrap()
        .to_vec();
    let other = Construction::new(incoming).unwrap();
    let before = counts(&h);
    assert!(system_factory::allocate(&mut h, other.request()).is_err());
    assert_eq!(counts(&h), before);
}

#[test]
fn system_base72_ack_replay_after_derived_stores_never_rewinds_full_images() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    finish(&mut c, &mut h);
    let cp = c.checkpoint().unwrap();
    let before = counts(&h);
    let call = c.base72_call().unwrap();
    let p = prefix(&c);
    assert_ne!(
        cp.parent(),
        p.parent(),
        "fixture did not advance derived source stores"
    );
    for _ in 0..2 {
        system_base72::acknowledge(&mut h, p).unwrap();
        system_factory::acknowledge(&mut h, call, p).unwrap();
        let request = c.request().clone();
        factory::Host::acknowledge_base72(&mut h.system_provider_host(&request), call, p).unwrap();
        assert_eq!(
            h.memory.allocation_bytes(cp.parent().token).unwrap(),
            cp.parent().data
        );
        assert_eq!(
            h.memory.allocation_bytes(cp.caller().token).unwrap(),
            cp.caller().data
        );
        assert_eq!(counts(&h), before);
        assert!(check_lookup(&h, &c).is_ok());
    }
}

#[test]
fn system_base72_historical_postallocation_checkpoint_cannot_rewind_later_source() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    let old = loop {
        step(&mut c, &mut h).unwrap();
        if let Ok(cp) = c.checkpoint() {
            if cp.initial_tree(TreeSite::Derived).is_some() {
                break cp;
            }
        }
    };
    finish(&mut c, &mut h);
    let current = c.checkpoint().unwrap();
    assert!(old.source_ordinal() < current.source_ordinal());
    assert_ne!(old.parent(), current.parent());
    let before = counts(&h);
    assert!(system_factory::synchronize(&mut h, &old).is_err());
    let request = c.request().clone();
    assert!(
        factory::Host::synchronize_checkpoint(&mut h.system_provider_host(&request), &old).is_err()
    );
    assert_eq!(
        h.memory.allocation_bytes(current.parent().token).unwrap(),
        current.parent().data
    );
    assert_eq!(
        h.memory.allocation_bytes(current.caller().token).unwrap(),
        current.caller().data
    );
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        let tree = current.tree(site).unwrap();
        assert_eq!(h.memory.allocation_bytes(tree.token).unwrap(), tree.data);
    }
    system_factory::synchronize(&mut h, &current).unwrap();
    system_factory::synchronize(&mut h, &current).unwrap();
    assert_eq!(counts(&h), before);
    assert!(check_lookup(&h, &c).is_ok());
}

struct Lost<'s, 'a, 'b> {
    inner: SystemProviderHost<'s, 'a>,
    lost: &'b mut bool,
}
impl factory::Host for Lost<'_, '_, '_> {
    fn allocate_provider(&mut self, r: &factory::Request) -> Result<Allocated, Stop> {
        factory::Host::allocate_provider(&mut self.inner, r)
    }
    fn allocate_tree(&mut self, r: &TreeRequest) -> Result<Allocated, Stop> {
        factory::Host::allocate_tree(&mut self.inner, r)
    }
    fn prepare_base72(&mut self, r: &Checkpoint) -> Result<(), Stop> {
        factory::Host::prepare_base72(&mut self.inner, r)
    }
    fn initialize_base72(&mut self, r: &Base72Call) -> Result<InitializedSection, Stop> {
        let reply = factory::Host::initialize_base72(&mut self.inner, r)?;
        if !*self.lost {
            *self.lost = true;
            Err(Stop::Unavailable(
                "actual staged reply handoff lost once".into(),
            ))
        } else {
            Ok(reply)
        }
    }
    fn acknowledge_base72(&mut self, c: &Base72Call, p: &base72::Prefix) -> Result<(), Stop> {
        factory::Host::acknowledge_base72(&mut self.inner, c, p)
    }
    fn synchronize_checkpoint(&mut self, c: &Checkpoint) -> Result<(), Stop> {
        factory::Host::synchronize_checkpoint(&mut self.inner, c)
    }
}

#[test]
fn system_base72_actual_staged_reply_survives_failed_handoff_without_duplicate_resources() {
    let (i, d) = fixture();
    let mut h = host(&i, &d, 8192);
    let mut c = cursor(&mut h);
    let mut lost = false;
    let mut saw_error = false;
    let mut done = false;
    for _ in 0..512 {
        let request = c.request().clone();
        match c.step(&mut Lost {
            inner: h.system_provider_host(&request),
            lost: &mut lost,
        }) {
            Ok(Some(Cut::FirstDerivedChildReturned)) => {
                done = true;
                break;
            }
            Ok(_) => {}
            Err(error) => {
                assert!(matches!(error, Stop::Unavailable(_)));
                assert!(lost);
                assert!(!saw_error);
                saw_error = true;
                assert_eq!(h.live_capabilities(), 2);
            }
        }
    }
    assert!(done && saw_error && lost);
    assert_eq!((h.memory.live_allocations(), h.live_capabilities()), (5, 2));
    assert!(check_lookup(&h, &c).is_ok());
    c.checkpoint().unwrap().validate().unwrap();
}
