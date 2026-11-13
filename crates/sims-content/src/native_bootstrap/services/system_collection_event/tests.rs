use super::super::super::system_collection_event as backend;
use super::*;
use sims_kernel::event::{Error as EventError, Event, Outcome, ResetMode, Timeout};
use sims_sim::native_system::collection::event as source;

fn ready(h: &mut BootstrapServices<'_>) -> collection::Construction {
    let mut c = cursor(h);
    finish_parent(&mut c, h);
    c
}
fn finish_event(
    c: &mut collection::Construction,
    h: &mut BootstrapServices<'_>,
) -> source::Checkpoint {
    for _ in 0..16 {
        let request = c.request().clone();
        if c.event_step(&mut h.system_collection_event_host(&request))
            .unwrap()
            == Some(source::Cut::BeforeCounter)
        {
            return c.event_checkpoint().unwrap().unwrap();
        }
    }
    panic!("actual event source did not reach counter boundary");
}
fn returned(c: &collection::Construction) -> source::Return {
    c.event_checkpoint()
        .unwrap()
        .unwrap()
        .returned()
        .unwrap()
        .clone()
}
fn event_key(h: &BootstrapServices<'_>) -> u32 {
    h.capabilities
        .iter()
        .find_map(|(key, cap)| matches!(cap, Capability::SystemCollectionEvent(_)).then_some(*key))
        .unwrap()
}

#[test]
fn event_backend_owns_the_actual_event_return_and_preserves_all_predecessor_resources() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    let before = counts(&h);
    let cut = finish_event(&mut c, &mut h);
    let r = returned(&c);
    let after = counts(&h);
    assert_eq!((before.0, before.1), (after.0, after.1));
    assert_eq!(after.2, before.2 + 1);
    assert_eq!(after.3, before.3 + 1);
    assert_ne!(r.handle(), 0);
    assert_eq!(
        u32::from_le_bytes(cut.parent().data[260..264].try_into().unwrap()),
        r.handle()
    );
    assert_eq!(
        h.memory.allocation_bytes(cut.parent().token).unwrap(),
        cut.parent().data
    );
    assert_eq!(
        h.collection_event(&r)
            .unwrap()
            .begin_wait(Timeout::Milliseconds(0))
            .unwrap()
            .wait(),
        Ok(Outcome::TimedOut)
    );
    for section in cut.call().prefix().sections().iter().flatten() {
        assert!(
            h.section(section.capability, section.region.token, 24)
                .is_ok()
        );
    }
    let first = c.request().factory().base72().unwrap().section();
    assert!(h.section(first.capability, first.region.token, 24).is_ok());
    assert!(cut.before_counter());
    let before = counts(&h);
    let request = c.request().clone();
    assert!(
        c.event_step(&mut h.system_collection_event_host(&request))
            .is_err()
    );
    assert_eq!(counts(&h), before);
}

struct Lost<'s, 'a> {
    h: &'s mut BootstrapServices<'a>,
    request: Request,
    held: Option<source::Return>,
    once: bool,
}
impl source::Host for Lost<'_, '_> {
    fn create_event(&mut self, call: &source::Call) -> Result<source::Return, Stop> {
        let r = source::Host::create_event(
            &mut self.h.system_collection_event_host(&self.request),
            call,
        )?;
        self.held = Some(r.clone());
        if self.once {
            self.once = false;
            return Err(Stop::Unavailable("authored lost event delivery".into()));
        }
        Ok(r)
    }
    fn synchronize_event(&mut self, cut: &source::Checkpoint) -> Result<(), Stop> {
        source::Host::synchronize_event(
            &mut self.h.system_collection_event_host(&self.request),
            cut,
        )
    }
}

#[test]
fn event_backend_lost_delivery_keeps_one_live_pending_effect_without_fake_source_ack() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    let before = counts(&h);
    let request = c.request().clone();
    let mut host = Lost {
        h: &mut h,
        request,
        held: None,
        once: true,
    };
    assert!(c.event_step(&mut host).is_err());
    let actual = host.held.clone().unwrap();
    drop(host);
    let root = root_for_event(&h, c.request());
    assert!(root.event().unwrap().admitted.is_none());
    assert!(c.event_checkpoint().unwrap().unwrap().returned().is_none());
    h.collection_event(&actual).unwrap().check_live().unwrap();
    finish_event(&mut c, &mut h);
    assert!(returned(&c).same_receipt(&actual));
    assert_eq!(counts(&h).2, before.2 + 1);
}
fn root_for_event<'s>(h: &'s BootstrapServices<'_>, request: &Request) -> &'s Binding {
    h.capabilities
        .values()
        .find_map(|cap| match cap {
            Capability::SystemCollection(root) if root.request().same_call(request) => Some(root),
            _ => None,
        })
        .unwrap()
}

#[test]
fn event_lookup_and_duplicate_creation_do_not_consume_or_reset_a_real_signal() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    finish_event(&mut c, &mut h);
    let r = returned(&c);
    let before = counts(&h);
    h.collection_event(&r).unwrap().set().unwrap();
    let again = backend::create(&mut h, r.call()).unwrap();
    assert!(again.same_receipt(&r));
    h.collection_event(&r).unwrap().check_live().unwrap();
    h.collection_event(&again).unwrap().check_live().unwrap();
    assert_eq!(
        h.collection_event(&r)
            .unwrap()
            .begin_wait(Timeout::Milliseconds(0))
            .unwrap()
            .wait(),
        Ok(Outcome::Signaled)
    );
    assert_eq!(
        h.collection_event(&r)
            .unwrap()
            .begin_wait(Timeout::Milliseconds(0))
            .unwrap()
            .wait(),
        Ok(Outcome::TimedOut)
    );
    assert_eq!(counts(&h), before);
}

#[test]
fn event_quota_error_precedes_creation_and_retry_never_becomes_native_null() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    let next = h.next;
    h.next = CAP_START + MAX_CAPABILITIES;
    let before = counts(&h);
    let request = c.request().clone();
    assert!(matches!(
        c.event_step(&mut h.system_collection_event_host(&request)),
        Err(Stop::Unavailable(_))
    ));
    assert!(root_for_event(&h, c.request()).event().is_none());
    assert_eq!(counts(&h), before);
    h.next = next;
    finish_event(&mut c, &mut h);
    assert_ne!(returned(&c).handle(), 0);
}

#[test]
fn event_first_foreign_heap_is_rejected_before_a_new_capability() {
    let (i, d) = fixture();
    let mut issuing = services(&i, &d);
    let mut c = ready(&mut issuing);
    let mut foreign = services(&i, &d);
    let other = ready(&mut foreign);
    assert_eq!(
        c.checkpoint().unwrap().parent(),
        other.checkpoint().unwrap().parent()
    );
    let before = counts(&foreign);
    let request = c.request().clone();
    assert!(
        c.event_step(&mut foreign.system_collection_event_host(&request))
            .is_err()
    );
    assert_eq!(counts(&foreign), before);
    assert!(root_for_event(&foreign, other.request()).event().is_none());
}

struct Unacked<'s, 'a> {
    h: &'s mut BootstrapServices<'a>,
    request: Request,
}
impl collection::Host for Unacked<'_, '_> {
    fn synchronize_checkpoint(&mut self, cut: &Checkpoint) -> Result<(), Stop> {
        if cut.source_ordinal() == 39 {
            return Ok(());
        }
        collection::Host::synchronize_checkpoint(
            &mut self.h.system_collection_host(&self.request),
            cut,
        )
    }
    fn allocate_list(&mut self, call: &ListRequest) -> Result<Allocated, Stop> {
        collection::Host::allocate_list(&mut self.h.system_collection_host(&self.request), call)
    }
    fn initialize_section(&mut self, call: &SectionCall) -> Result<InitializedSection, Stop> {
        collection::Host::initialize_section(
            &mut self.h.system_collection_host(&self.request),
            call,
        )
    }
}

#[test]
fn event_valid_cached_cut_cannot_replace_the_actual_unsynchronized_root_cut() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = cursor(&mut h);
    let request = c.request().clone();
    let mut cut = false;
    for _ in 0..256 {
        if c.step(&mut Unacked {
            h: &mut h,
            request: request.clone(),
        })
        .unwrap()
            == Some(collection::Cut::BeforeResource)
        {
            cut = true;
            break;
        }
    }
    assert!(cut);
    let before = counts(&h);
    assert!(
        c.event_step(&mut h.system_collection_event_host(&request))
            .is_err()
    );
    assert_eq!(counts(&h), before);
    assert!(root_for_event(&h, &request).event().is_none());
}

#[test]
fn event_missing_capability_is_sticky_and_cannot_be_recreated_on_lost_reply_retry() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    let request = c.request().clone();
    let mut host = Lost {
        h: &mut h,
        request: request.clone(),
        held: None,
        once: true,
    };
    assert!(c.event_step(&mut host).is_err());
    let r = host.held.clone().unwrap();
    drop(host);
    let key = event_key(&h);
    let removed = h.capabilities.remove(&key).unwrap();
    let before = counts(&h);
    assert!(h.collection_event(&r).is_err());
    assert!(
        c.event_step(&mut h.system_collection_event_host(&request))
            .is_err()
    );
    assert_eq!(counts(&h), before);
    drop(removed);
}

#[test]
fn event_moved_capability_or_equal_new_event_cannot_replace_the_issued_lifetime() {
    for replace in [false, true] {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = ready(&mut h);
        finish_event(&mut c, &mut h);
        let r = returned(&c);
        let key = event_key(&h);
        if replace {
            let Some(Capability::SystemCollectionEvent(entry)) = h.capabilities.get_mut(&key)
            else {
                panic!("event");
            };
            entry.event = Event::new(ResetMode::Auto, false);
        } else {
            let entry = h.capabilities.remove(&key).unwrap();
            h.capabilities.insert(key + 100, entry);
        }
        let before = counts(&h);
        assert!(h.collection_event(&r).is_err());
        assert!(backend::create(&mut h, r.call()).is_err());
        assert_eq!(counts(&h), before);
    }
}

#[test]
fn event_forged_same_handle_receipt_does_not_acquire_the_actual_resource() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    finish_event(&mut c, &mut h);
    let r = returned(&c);
    let forged = source::Return::from_host(r.call(), u64::from(r.handle())).unwrap();
    assert!(!r.same_receipt(&forged));
    assert!(h.collection_event(&forged).is_err());
    assert!(h.collection_event(&r).is_ok());
}

#[test]
fn event_retired_real_owner_invalidates_access_and_creation_retry() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    finish_event(&mut c, &mut h);
    let r = returned(&c);
    let key = event_key(&h);
    let Some(Capability::SystemCollectionEvent(entry)) = h.capabilities.get_mut(&key) else {
        panic!("event");
    };
    entry.event.retire().unwrap();
    let before = counts(&h);
    assert!(h.collection_event(&r).is_err());
    assert!(backend::create(&mut h, r.call()).is_err());
    assert_eq!(counts(&h), before);
}

#[test]
fn event_source_ownership_fences_the_identical_old_prefix_before_its_fast_path() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    let prefix = c.checkpoint().unwrap();
    let request = c.request().clone();
    c.event_step(&mut h.system_collection_event_host(&request))
        .unwrap();
    let before = counts(&h);
    assert!(synchronize(&mut h, &prefix).is_err());
    assert_eq!(counts(&h), before);
    finish_event(&mut c, &mut h);
    let data = h
        .memory
        .allocation_bytes(prefix.parent().token)
        .unwrap()
        .to_vec();
    assert!(synchronize(&mut h, &prefix).is_err());
    assert_eq!(
        h.memory.allocation_bytes(prefix.parent().token).unwrap(),
        data
    );
    assert!(system_factory::synchronize(&mut h, request.factory()).is_err());
}

#[test]
fn event_stale_suffix_checkpoint_cannot_rewind_later_parent_stores() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    let request = c.request().clone();
    c.event_step(&mut h.system_collection_event_host(&request))
        .unwrap();
    let old = c.event_checkpoint().unwrap().unwrap();
    finish_event(&mut c, &mut h);
    let before = h
        .memory
        .allocation_bytes(old.parent().token)
        .unwrap()
        .to_vec();
    assert!(
        source::Host::synchronize_event(&mut h.system_collection_event_host(&request), &old)
            .is_err()
    );
    assert_eq!(
        h.memory.allocation_bytes(old.parent().token).unwrap(),
        before
    );
}

#[test]
fn event_preflight_refuses_and_does_not_repair_tampered_shared_dependency() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    let request = c.request().clone();
    c.event_step(&mut h.system_collection_event_host(&request))
        .unwrap();
    let r = returned(&c);
    let first = request.factory().base72().unwrap().section().capability as u32;
    let Some(Capability::SystemBase72(entry)) = h.capabilities.get_mut(&first) else {
        panic!("first");
    };
    entry.parent_image.data[0] ^= 1;
    let changed = entry.parent_image.clone();
    let before = counts(&h);
    assert!(
        source::Host::create_event(&mut h.system_collection_event_host(&request), r.call())
            .is_err()
    );
    let Some(Capability::SystemBase72(entry)) = h.capabilities.get(&first) else {
        panic!("first");
    };
    assert_eq!(entry.parent_image, changed);
    assert_eq!(counts(&h), before);
}

#[test]
fn event_dropping_services_retires_an_actual_pending_wait_without_a_detached_owner() {
    let (i, d) = fixture();
    let mut h = services(&i, &d);
    let mut c = ready(&mut h);
    finish_event(&mut c, &mut h);
    let r = returned(&c);
    let mut waiting = h
        .collection_event(&r)
        .unwrap()
        .begin_wait(Timeout::Infinite)
        .unwrap();
    drop(h);
    assert_eq!(waiting.poll(), Err(EventError::Retired));
}

#[test]
fn event_allocation_release_or_lost_factory_invalidates_the_dependent_event() {
    for factory in [false, true] {
        let (i, d) = fixture();
        let mut h = services(&i, &d);
        let mut c = ready(&mut h);
        finish_event(&mut c, &mut h);
        let r = returned(&c);
        if factory {
            let key = h
                .capabilities
                .iter()
                .find_map(|(k, v)| matches!(v, Capability::SystemFactory(_)).then_some(*k))
                .unwrap();
            h.capabilities.remove(&key);
        } else {
            h.memory
                .release_owner(c.request().factory().parent().token)
                .unwrap();
        }
        assert!(h.collection_event(&r).is_err());
    }
}
