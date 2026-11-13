//! Authored integration coverage for the bounded Rust control context.
//! The broker fixture is synthetic; these tests make no native FPU claim.
use super::mutex_tests::{Authority, connected, entry, fpu as pre_read};
use super::*;
use sims_sim::native_application::{self, fpu};

fn install(raw: u16, exceptions: fpu::ExceptionState) -> fpu::Port {
    fpu::Port::install(fpu::InitialState {
        control_word: raw,
        exceptions,
    })
    .unwrap()
}

fn prepared(host: &mut BootstrapServices<'_>) -> Construction {
    let mut c = Construction::new(input(host)).unwrap();
    entry(&mut c, host);
    pre_read(&mut c, host);
    c
}

fn request(c: &Construction) -> fpu::ReadRequest {
    let request = c
        .pending_fpu_read_request()
        .expect("pending sealed FPU request");
    request.validate().unwrap();
    request
}

fn load_entry(c: &mut Construction, host: &mut BootstrapServices<'_>) -> fpu::LoadRequest {
    for _ in 0..64 {
        if let Some(Call::LoadControlWord(request)) = c.pending_call() {
            return request;
        }
        c.step(&mut host.application_host()).unwrap();
    }
    panic!("changed control word never reached FLDCW");
}

fn provider(c: &mut Construction, host: &mut BootstrapServices<'_>) {
    for _ in 0..64 {
        if matches!(
            c.pending_call(),
            Some(Call::ProviderVirtual {
                instruction: 0x4eee29
            })
        ) {
            return;
        }
        c.step(&mut host.application_host()).unwrap();
    }
    panic!("FPU continuation never reached the unsupported provider boundary");
}

fn read_binding<'h, 'a>(
    host: &'h mut BootstrapServices<'a>,
    request: &fpu::ReadRequest,
) -> &'h mut application_fpu::ReadBinding {
    host.capabilities
        .values_mut()
        .find_map(|cap| match cap {
            Capability::FpuRead(binding) if binding.request == *request => Some(binding),
            _ => None,
        })
        .expect("retained read binding")
}

fn load_binding<'h, 'a>(
    host: &'h mut BootstrapServices<'a>,
    request: &fpu::LoadRequest,
) -> &'h mut application_fpu::LoadBinding {
    host.capabilities
        .values_mut()
        .find_map(|cap| match cap {
            Capability::FpuLoad(binding) if binding.request == *request => Some(binding),
            _ => None,
        })
        .expect("retained load binding")
}

fn dword(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

#[test]
fn explicit_words_follow_both_branches_and_preserve_original_word_and_exact_store_order() {
    let authority = Authority::new();
    for (raw, after, changed) in [
        (0x037f, 0x007f, true),
        (0x003f, 0x003f, false),
        (0x007f, 0x007f, false),
        (0x003e, 0x003f, true),
        (0x0c3f, 0x003f, true),
        (0x817e, 0x807f, true),
    ] {
        let (index, dirs) = mounted(&[]);
        let mut host = connected(&index, &dirs, &authority, 5);
        let port = install(raw, fpu::ExceptionState::Flags(0));
        host.configure_application_fpu(port.clone()).unwrap();
        let mut c = prepared(&mut host);
        let before_callers = c.retained_callers().to_vec();
        let before_caps = host.live_capabilities();
        provider(&mut c, &mut host);
        let read = c.fpu_read().unwrap();
        assert_eq!(read.raw(), raw);
        read.validate(read.request()).unwrap();
        assert_eq!(c.fpu_load().is_some(), changed);
        if let Some(load) = c.fpu_load() {
            assert_eq!((load.before(), load.after()), (raw, after));
            assert_eq!(load.serial(), read.serial() + 1);
            load.validate(load.request()).unwrap();
        }
        assert_eq!(&c.owner().data[254..256], &raw.to_le_bytes());
        assert_eq!(dword(&c.owner().data[256..]), u32::from(changed));
        assert_eq!((c.owner().data[260], c.owner().data[261]), (0, 1));
        assert_eq!(dword(&c.owner().data[176..]), 0);
        assert_eq!(dword(&c.owner().data[264..]), 0);
        assert_eq!(
            host.live_capabilities(),
            before_caps + 1 + usize::from(changed)
        );
        let mut expected = vec![0x4eedc7];
        if changed {
            expected.push(0x4eedf1);
        }
        expected.extend([0x4eedff, 0x4eee02, 0x4eee08, 0x4eee0e, 0x4eee14, 0x4eee1b]);
        let writes: Vec<_> = c
            .writes()
            .iter()
            .filter(|w| (0x4eedc7..=0x4eee1b).contains(&w.instruction))
            .collect();
        assert_eq!(
            writes.iter().map(|w| w.instruction).collect::<Vec<_>>(),
            expected
        );
        let read_store = writes[0];
        let old_caller = before_callers
            .iter()
            .find(|o| o.token == read_store.owner)
            .unwrap();
        let caller = c
            .retained_callers()
            .iter()
            .find(|o| o.token == read_store.owner)
            .unwrap();
        let mut expected_caller = old_caller.clone();
        expected_caller.data[read_store.offset..read_store.offset + 2]
            .copy_from_slice(&raw.to_le_bytes());
        assert_eq!(read_store.after, raw.to_le_bytes()); // FNSTCW writes only a WORD
        assert_eq!(
            &caller.data[read_store.offset + 2..read_store.offset + 4],
            &old_caller.data[read_store.offset + 2..read_store.offset + 4]
        );
        assert_eq!(dword(&caller.data[read_store.offset..]), u32::from(raw));
        if changed {
            let normalized_store = writes.iter().find(|w| w.instruction == 0x4eedf1).unwrap();
            assert_eq!(normalized_store.owner, caller.token);
            assert_eq!(normalized_store.offset, read_store.offset + 26); // [ebp+a]
            assert_eq!(normalized_store.after, after.to_le_bytes());
            // The WORD store aliases the high half of the original argument
            // DWORD at [ebp+8], without replacing its low half or its owner.
            let at = normalized_store.offset;
            expected_caller.data[at..at + 2].copy_from_slice(&after.to_le_bytes());
            assert_eq!(&caller.data[at - 2..at], &old_caller.data[at - 2..at]);
            assert_eq!(&caller.data[at + 2..], &old_caller.data[at + 2..]);
        }
        assert_eq!(caller, &expected_caller);
        assert_eq!(host.application_fpu_control(&c).unwrap(), after);
        assert_eq!(
            host.memory.allocation_bytes(c.owner().token).unwrap(),
            c.owner().data
        );
        assert_eq!(port.control_word().unwrap(), after);
        let owner = c.owner().clone();
        let writes = c.writes().to_vec();
        assert!(matches!(
            c.step(&mut host.application_host()),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(c.owner(), &owner);
        assert_eq!(c.writes(), writes);
        c.validate().unwrap();
    }
}

#[test]
fn missing_configuration_is_retryable_but_never_installs_an_implicit_context() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut c = prepared(&mut host);
    let request = request(&c);
    let next = host.next;
    let owner = host
        .memory
        .allocation_bytes(c.owner().token)
        .unwrap()
        .to_vec();
    for _ in 0..3 {
        assert!(matches!(
            host.read_application_control(&request),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(host.next, next);
        assert_eq!(host.live_capabilities(), 3);
        assert_eq!(
            host.memory.allocation_bytes(c.owner().token).unwrap(),
            owner
        );
        assert!(c.fpu_read().is_none());
        c.validate().unwrap();
    }
    // Installation succeeds here only if the failed requests installed nothing.
    let port = install(0x003f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    assert!(host.configure_application_fpu(port.clone()).is_err());
    provider(&mut c, &mut host);
    assert_eq!(c.fpu_read().unwrap().raw(), 0x003f);
    assert_eq!(host.live_capabilities(), 4);
}

#[test]
fn unknown_or_pending_unmasked_wait_retains_one_pending_slot_and_no_fabricated_return() {
    let authority = Authority::new();
    for (raw, exceptions) in [
        (0x003f, fpu::ExceptionState::Unknown),
        (0x003e, fpu::ExceptionState::Flags(1)),
    ] {
        let (index, dirs) = mounted(&[]);
        let mut host = connected(&index, &dirs, &authority, 5);
        let port = install(raw, exceptions);
        host.configure_application_fpu(port.clone()).unwrap();
        let mut c = prepared(&mut host);
        let request = request(&c);
        let owner = c.owner().clone();
        let writes = c.writes().to_vec();
        let next = host.next;
        for _ in 0..3 {
            assert!(matches!(
                host.read_application_control(&request),
                Err(Stop::Unavailable(_))
            ));
            assert!(matches!(
                c.step(&mut host.application_host()),
                Err(Stop::Unavailable(_))
            ));
            assert_eq!(host.next, next + 1);
            assert_eq!(host.live_capabilities(), 4);
            assert!(read_binding(&mut host, &request).returned.is_none());
            assert!(c.fpu_read().is_none());
            assert!(c.fpu_load().is_none());
            assert_eq!(c.owner(), &owner);
            assert_eq!(c.writes(), writes);
            assert_eq!(port.control_word().unwrap(), raw);
            c.validate().unwrap();
        }
    }
}

#[test]
fn masked_pending_flags_allow_a_real_read_and_normalized_load() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0x3f));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut c = prepared(&mut host);
    provider(&mut c, &mut host);
    assert_eq!(c.fpu_read().unwrap().raw(), 0x037f);
    assert_eq!(c.fpu_load().unwrap().after(), 0x007f);
    assert_eq!(host.application_fpu_control(&c).unwrap(), 0x007f);
    let mut d = prepared(&mut host);
    provider(&mut d, &mut host);
    assert_eq!(d.fpu_read().unwrap().raw(), 0x007f);
    assert!(d.fpu_load().is_none());
}

#[test]
fn unchanged_capability_budget_is_reserved_before_read_or_load_effects() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut c = prepared(&mut host);
    let request = request(&c);
    let next = host.next;
    let image = host
        .memory
        .allocation_bytes(c.owner().token)
        .unwrap()
        .to_vec();
    host.next = CAP_START + MAX_CAPABILITIES;
    for _ in 0..2 {
        assert!(matches!(
            host.read_application_control(&request),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(host.live_capabilities(), 3);
        assert_eq!(
            host.memory.allocation_bytes(c.owner().token).unwrap(),
            image
        );
        assert_eq!(port.control_word().unwrap(), 0x037f);
        assert!(c.fpu_read().is_none());
    }
    // Only these tests restore an artificial exhaustion of the monotonic quota.
    host.next = next;
    let load = load_entry(&mut c, &mut host);
    let writes = c.writes().to_vec();
    let next = host.next;
    host.next = CAP_START + MAX_CAPABILITIES;
    for _ in 0..2 {
        assert!(matches!(
            host.load_application_control(&load),
            Err(Stop::Unavailable(_))
        ));
        assert!(matches!(
            c.step(&mut host.application_host()),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(port.control_word().unwrap(), 0x037f);
        assert_eq!(host.live_capabilities(), 4);
        assert!(c.fpu_load().is_none());
        assert_eq!(c.writes(), writes);
        assert!(!c.writes().iter().any(|w| w.instruction == 0x4eedff));
        c.validate().unwrap();
    }
    host.next = next;
    provider(&mut c, &mut host);
    assert_eq!(host.live_capabilities(), 5);
    assert_eq!(port.control_word().unwrap(), 0x007f);
}

#[test]
fn cached_retry_never_allocates_or_repeats_a_returned_operation_even_at_quota() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut c = prepared(&mut host);
    let read_request = request(&c);
    let read = host.read_application_control(&read_request).unwrap();
    let load_request = load_entry(&mut c, &mut host);
    assert_eq!(c.fpu_read(), Some(&read));
    let load = host.load_application_control(&load_request).unwrap();
    let next = host.next;
    host.next = CAP_START + MAX_CAPABILITIES;
    for _ in 0..3 {
        assert_eq!(host.read_application_control(&read_request).unwrap(), read);
        assert_eq!(host.load_application_control(&load_request).unwrap(), load);
        assert_eq!(host.live_capabilities(), 5);
        assert_eq!(port.control_word().unwrap(), 0x007f);
    }
    provider(&mut c, &mut host);
    assert_eq!(host.application_fpu_control(&c).unwrap(), 0x007f);
    assert_eq!(c.fpu_read(), Some(&read));
    assert_eq!(c.fpu_load(), Some(&load));
    // Admission of current stores updates records. A retry must not restore
    // request.application_image(), which predates those stores.
    let image = c.owner().clone();
    assert_eq!(host.read_application_control(&read_request).unwrap(), read);
    assert_eq!(host.load_application_control(&load_request).unwrap(), load);
    assert_eq!(
        host.memory.allocation_bytes(image.token).unwrap(),
        image.data
    );
    let fresh = port.read_control(&read_request).unwrap();
    assert_eq!((fresh.raw(), fresh.serial()), (0x007f, load.serial() + 1));
    assert_eq!(host.next, CAP_START + MAX_CAPABILITIES);
    assert_eq!(next, CAP_START + 5);
}

#[test]
fn same_thread_services_share_only_the_explicit_context_and_keep_distinct_issuance() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut first = connected(&index, &dirs, &authority, 5);
    let mut second = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    first.configure_application_fpu(port.clone()).unwrap();
    second.configure_application_fpu(port.clone()).unwrap();
    let mut a = prepared(&mut first);
    let mut b = prepared(&mut second);
    assert_eq!(a.owner().token, b.owner().token);
    let ar = request(&a);
    let br = request(&b);
    assert_ne!(ar, br);
    assert!(first.read_application_control(&br).is_err());
    assert!(second.read_application_control(&ar).is_err());
    assert_eq!(
        (first.live_capabilities(), second.live_capabilities()),
        (3, 3)
    );
    provider(&mut a, &mut first);
    provider(&mut b, &mut second);
    assert_eq!(a.fpu_read().unwrap().raw(), 0x037f);
    assert_eq!(b.fpu_read().unwrap().raw(), 0x007f);
    assert_ne!(a.fpu_read(), b.fpu_read());
    assert!(port.owns_read(a.fpu_read().unwrap()));
    assert!(port.owns_read(b.fpu_read().unwrap()));
    assert!(
        second
            .load_application_control(a.fpu_load().unwrap().request())
            .is_err()
    );
    assert!(first.application_fpu_control(&b).is_err());
    assert!(second.application_fpu_control(&a).is_err());
    assert_eq!(first.application_fpu_control(&a).unwrap(), 0x007f);
    assert_eq!(second.application_fpu_control(&b).unwrap(), 0x007f);
}

#[test]
fn pre_read_admission_rejects_unrecorded_heap_images_before_sync_or_port_effect() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x003f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let c = prepared(&mut host);
    let request = request(&c);
    let probe = port.read_control(&request).unwrap();
    let heap_app = Owner {
        token: c.owner().token,
        data: host
            .memory
            .allocation_bytes(c.owner().token)
            .unwrap()
            .to_vec(),
    };
    let mut altered = heap_app.clone();
    altered.data[303] ^= 1;
    host.memory.synchronize_owner(&altered).unwrap();
    let next = host.next;
    for _ in 0..2 {
        assert!(host.read_application_control(&request).is_err());
        assert_eq!(
            host.memory.allocation_bytes(altered.token).unwrap(),
            altered.data
        );
        assert_eq!(host.next, next);
    }
    host.memory.synchronize_owner(&heap_app).unwrap();
    let wrapper = c.mutex_wrapper().unwrap();
    let heap_wrapper = Owner {
        token: wrapper.token,
        data: host
            .memory
            .allocation_bytes(wrapper.token)
            .unwrap()
            .to_vec(),
    };
    let mut altered = heap_wrapper.clone();
    altered.data[11] ^= 1;
    host.memory.synchronize_owner(&altered).unwrap();
    assert!(host.read_application_control(&request).is_err());
    assert_eq!(
        host.memory.allocation_bytes(altered.token).unwrap(),
        altered.data
    );
    assert_eq!(host.next, next);
    host.memory.synchronize_owner(&heap_wrapper).unwrap();
    let read = host.read_application_control(&request).unwrap();
    assert_eq!(read.serial(), probe.serial() + 1);
    assert_eq!(host.live_capabilities(), 4);
    c.validate().unwrap();
}

#[test]
fn caller_byte_views_alias_the_control_word_without_retroactively_changing_copied_bytes() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port).unwrap();
    let mut incoming = input(&mut host);
    let caller = incoming.mutex_temporary - 16;
    incoming.tree_stack = [caller + 44, caller + 45];
    let mut c = Construction::new(incoming).unwrap();
    entry(&mut c, &mut host);
    pre_read(&mut c, &mut host);
    // The source zeroed the DWORD before the tree copied its low byte.
    assert_eq!(c.owner().data[88], 0);
    provider(&mut c, &mut host);
    let containing = c
        .retained_callers()
        .iter()
        .find(|o| o.token == caller)
        .unwrap();
    assert_eq!(&containing.data[44..48], &[0x7f, 0x03, 0, 0]);
    assert_eq!(c.owner().data[88], 0);
    assert_eq!(&containing.data[70..72], &[0x7f, 0]);
    c.validate().unwrap();
    assert_eq!(host.application_fpu_control(&c).unwrap(), 0x007f);
}

#[test]
fn identical_addresses_and_images_do_not_authorize_an_in_place_heap_swap() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut first = connected(&index, &dirs, &authority, 5);
    let mut second = connected(&index, &dirs, &authority, 5);
    let port = install(0x003f, fpu::ExceptionState::Flags(0));
    first.configure_application_fpu(port.clone()).unwrap();
    second.configure_application_fpu(port.clone()).unwrap();
    let mut a = prepared(&mut first);
    let mut b = prepared(&mut second);
    provider(&mut a, &mut first);
    provider(&mut b, &mut second);
    first.application_fpu_control(&a).unwrap();
    second.application_fpu_control(&b).unwrap();
    assert_eq!(a.owner(), b.owner());
    let ar = a.fpu_read().unwrap().clone();
    let br = b.fpu_read().unwrap().clone();
    std::mem::swap(first.memory_mut(), second.memory_mut());
    for _ in 0..2 {
        assert!(first.application_fpu_control(&a).is_err());
        assert!(second.application_fpu_control(&b).is_err());
        assert!(first.read_application_control(ar.request()).is_err());
        assert!(second.read_application_control(br.request()).is_err());
        assert_eq!(
            (first.live_capabilities(), second.live_capabilities()),
            (4, 4)
        );
        a.validate().unwrap();
        b.validate().unwrap();
    }
    std::mem::swap(first.memory_mut(), second.memory_mut());
    assert_eq!(first.application_fpu_control(&a).unwrap(), 0x003f);
}

#[test]
fn full_application_wrapper_and_released_owner_checks_precede_load_and_fresh_access() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut c = prepared(&mut host);
    let load = load_entry(&mut c, &mut host);
    let next = host.next;
    let mut altered = c.owner().clone();
    altered.data[303] ^= 1;
    host.memory.synchronize_owner(&altered).unwrap();
    assert!(host.load_application_control(&load).is_err());
    assert_eq!(host.next, next);
    assert_eq!(port.control_word().unwrap(), 0x037f);
    c.validate().unwrap();
    host.memory.synchronize_owner(c.owner()).unwrap();
    let mut altered_wrapper = c.mutex_wrapper().unwrap().clone();
    altered_wrapper.data[11] ^= 1;
    host.memory.synchronize_owner(&altered_wrapper).unwrap();
    assert!(host.load_application_control(&load).is_err());
    assert_eq!(host.next, next);
    host.memory
        .synchronize_owner(c.mutex_wrapper().unwrap())
        .unwrap();
    provider(&mut c, &mut host);
    host.application_fpu_control(&c).unwrap();
    host.memory.release_owner(c.owner().token).unwrap();
    assert!(host.application_fpu_control(&c).is_err());
    assert!(host.load_application_control(&load).is_err());
    assert_eq!(host.live_capabilities(), 5);
    c.validate().unwrap();
    c.fpu_load().unwrap().validate(&load).unwrap();
}

#[test]
fn historical_replay_survives_later_load_retirement_and_a_new_explicit_context() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut a = prepared(&mut host);
    let mut b = prepared(&mut host);
    // Both reads precede the first load. The second load's real before value
    // must be current state, rather than the original value in its read.
    let al = load_entry(&mut a, &mut host);
    let bl = load_entry(&mut b, &mut host);
    provider(&mut a, &mut host);
    provider(&mut b, &mut host);
    assert_eq!(a.fpu_load().unwrap().before(), 0x037f);
    assert_eq!(b.fpu_load().unwrap().before(), 0x007f);
    assert_eq!(b.fpu_load().unwrap().after(), 0x007f);
    for c in [&a, &b] {
        c.validate().unwrap();
        assert_eq!(c.fpu_read().unwrap().raw(), 0x037f);
        assert_eq!(host.application_fpu_control(c).unwrap(), 0x007f);
    }
    port.retire().unwrap();
    let replacement = install(0x0c3f, fpu::ExceptionState::Flags(0));
    assert!(!replacement.owns_read(a.fpu_read().unwrap()));
    assert!(host.configure_application_fpu(replacement.clone()).is_err());
    let caps = host.live_capabilities();
    for (c, load) in [(&a, &al), (&b, &bl)] {
        for _ in 0..3 {
            c.validate().unwrap();
            c.fpu_read()
                .unwrap()
                .validate(c.fpu_read().unwrap().request())
                .unwrap();
            c.fpu_load().unwrap().validate(load).unwrap();
            assert!(host.application_fpu_control(c).is_err());
            assert!(
                host.read_application_control(c.fpu_read().unwrap().request())
                    .is_err()
            );
            assert!(host.load_application_control(load).is_err());
            assert_eq!(host.live_capabilities(), caps);
            assert_eq!(replacement.control_word().unwrap(), 0x0c3f);
        }
    }
    let mut foreign = connected(&index, &dirs, &authority, 5);
    foreign
        .configure_application_fpu(replacement.clone())
        .unwrap();
    assert!(foreign.load_application_control(&al).is_err());
    assert_eq!(foreign.live_capabilities(), 0);
}

#[test]
fn staged_malformed_read_and_load_bindings_remain_retained_without_port_repetition() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut a = prepared(&mut host);
    let mut b = prepared(&mut host);
    let ar = request(&a);
    let br = request(&b);
    let aread = host.read_application_control(&ar).unwrap();
    let bread = host.read_application_control(&br).unwrap();
    read_binding(&mut host, &ar).returned = Some(bread.clone());
    let next = host.next;
    for _ in 0..3 {
        assert!(host.read_application_control(&ar).is_err());
        assert_eq!(read_binding(&mut host, &ar).returned.as_ref(), Some(&bread));
        assert_eq!(host.next, next);
    }
    let fresh = port.read_control(&br).unwrap();
    assert_eq!(fresh.serial(), bread.serial() + 1);
    read_binding(&mut host, &ar).returned = Some(aread);
    let al = load_entry(&mut a, &mut host);
    let bl = load_entry(&mut b, &mut host);
    let aload = host.load_application_control(&al).unwrap();
    let bload = host.load_application_control(&bl).unwrap();
    load_binding(&mut host, &al).returned = Some(bload.clone());
    let next = host.next;
    for _ in 0..3 {
        assert!(host.load_application_control(&al).is_err());
        assert_eq!(load_binding(&mut host, &al).returned.as_ref(), Some(&bload));
        assert_eq!(host.next, next);
    }
    let fresh = port.read_control(&br).unwrap();
    assert_eq!(fresh.serial(), bload.serial() + 1);
    load_binding(&mut host, &al).returned = Some(aload);
}

struct Gate<'s, 'a> {
    inner: ApplicationHost<'s, 'a>,
    foreign_read: Option<fpu::ReadReturn>,
    foreign_load: Option<fpu::LoadReturn>,
    failed_com: bool,
    null_wrapper: bool,
    block_load: bool,
    reads: usize,
    loads: usize,
}

fn gate<'s, 'a>(host: &'s mut BootstrapServices<'a>) -> Gate<'s, 'a> {
    Gate {
        inner: host.application_host(),
        foreign_read: None,
        foreign_load: None,
        failed_com: false,
        null_wrapper: false,
        block_load: false,
        reads: 0,
        loads: 0,
    }
}

impl strings::Host for Gate<'_, '_> {
    fn allocate_string(&mut self, r: strings::Request) -> Result<Allocated, Stop> {
        strings::Host::allocate_string(&mut self.inner, r)
    }
    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        strings::Host::free_string(&mut self.inner, token)
    }
}

impl native_application::Host for Gate<'_, '_> {
    fn initialize_section(
        &mut self,
        r: &SectionRequest,
    ) -> Result<native_application::InitializedSection, Stop> {
        self.inner.initialize_section(r)
    }
    fn allocate_tree(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.inner.allocate_tree(bytes)
    }
    fn initialize_com(&mut self, r: &com::Request) -> Result<com::Return, Stop> {
        if !self.failed_com {
            return self.inner.initialize_com(r);
        }
        // A genuine declared failed HRESULT, issued into this exact heap's
        // ledger. The source ignores it; it must not require a live apartment.
        let host = &mut self.inner.services;
        let token = host.next_capability()?;
        let returned = com::Return::failed(r, 0x80004005)?;
        host.commit(
            token,
            Capability::Apartment {
                heap: host.memory.identity(),
                application: r.application,
                returned: returned.clone(),
            },
        );
        Ok(returned)
    }
    fn query_version(
        &mut self,
        r: &platform::VersionRequest,
    ) -> Result<platform::VersionReturn, Stop> {
        self.inner.query_version(r)
    }
    fn allocate_mutex_wrapper(&mut self, r: &platform::WrapperRequest) -> Result<Allocated, Stop> {
        if self.null_wrapper {
            r.validate()?;
            Ok(Allocated::Null)
        } else {
            self.inner.allocate_mutex_wrapper(r)
        }
    }
    fn create_mutex(
        &mut self,
        r: &platform::MutexRequest,
    ) -> Result<native_application::mutex::Return, Stop> {
        self.inner.create_mutex(r)
    }
    fn read_control(&mut self, r: &fpu::ReadRequest) -> Result<fpu::ReadReturn, Stop> {
        self.reads += 1;
        match &self.foreign_read {
            Some(returned) => Ok(returned.clone()),
            None => self.inner.read_control(r),
        }
    }
    fn load_control(&mut self, r: &fpu::LoadRequest) -> Result<fpu::LoadReturn, Stop> {
        self.loads += 1;
        if self.block_load {
            return Err(Stop::Unavailable(
                "test FLDCW acknowledgement pending".into(),
            ));
        }
        match &self.foreign_load {
            Some(returned) => Ok(returned.clone()),
            None => self.inner.load_control(r),
        }
    }
}

#[test]
fn source_stores_wait_for_a_distinct_acknowledged_load_and_validation_never_replays_it() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut c = prepared(&mut host);
    let mut gate = gate(&mut host);
    gate.block_load = true;
    for _ in 0..64 {
        if matches!(c.pending_call(), Some(Call::LoadControlWord(_))) {
            break;
        }
        c.step(&mut gate).unwrap();
    }
    assert!(matches!(c.pending_call(), Some(Call::LoadControlWord(_))));
    assert_eq!(gate.reads, 1);
    let writes = c.writes().to_vec();
    assert!(writes.iter().any(|w| w.instruction == 0x4eedf1));
    for _ in 0..2 {
        assert!(matches!(c.step(&mut gate), Err(Stop::Unavailable(_))));
        assert_eq!(c.writes(), writes);
        assert_eq!(port.control_word().unwrap(), 0x037f);
        assert!(c.fpu_load().is_none());
        assert!(!c.writes().iter().any(|w| w.instruction == 0x4eedff));
        c.validate().unwrap();
        assert_eq!(gate.reads, 1);
    }
    gate.block_load = false;
    for _ in 0..64 {
        if matches!(
            c.pending_call(),
            Some(Call::ProviderVirtual {
                instruction: 0x4eee29
            })
        ) {
            break;
        }
        c.step(&mut gate).unwrap();
    }
    assert!(matches!(
        c.pending_call(),
        Some(Call::ProviderVirtual {
            instruction: 0x4eee29
        })
    ));
    assert_eq!((gate.reads, gate.loads), (1, 3));
    for _ in 0..3 {
        c.validate().unwrap();
    }
    assert_eq!((gate.reads, gate.loads), (1, 3));
}

#[test]
fn foreign_read_return_is_staged_before_validation_and_never_requested_again() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x003f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut a = prepared(&mut host);
    let mut b = prepared(&mut host);
    provider(&mut b, &mut host);
    let foreign = b.fpu_read().unwrap().clone();
    let owner = a.owner().clone();
    let writes = a.writes().to_vec();
    let mut gate = gate(&mut host);
    gate.foreign_read = Some(foreign.clone());
    for _ in 0..3 {
        assert!(matches!(a.step(&mut gate), Err(Stop::Unsupported(_))));
        assert_eq!((gate.reads, gate.loads), (1, 0));
        assert_eq!(a.owner(), &owner);
        assert_eq!(a.writes(), writes);
        assert!(a.validate().is_err());
    }
    foreign.validate(foreign.request()).unwrap();
    assert_eq!(foreign.current_word().unwrap(), 0x003f);
}

#[test]
fn foreign_load_return_is_staged_before_validation_and_never_requested_again() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x037f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port.clone()).unwrap();
    let mut a = prepared(&mut host);
    let mut b = prepared(&mut host);
    load_entry(&mut a, &mut host);
    load_entry(&mut b, &mut host);
    provider(&mut b, &mut host);
    let foreign = b.fpu_load().unwrap().clone();
    let owner = a.owner().clone();
    let writes = a.writes().to_vec();
    let mut gate = gate(&mut host);
    gate.foreign_load = Some(foreign.clone());
    for _ in 0..3 {
        assert!(matches!(a.step(&mut gate), Err(Stop::Unsupported(_))));
        assert_eq!((gate.reads, gate.loads), (0, 1));
        assert_eq!(a.owner(), &owner);
        assert_eq!(a.writes(), writes);
        assert!(a.validate().is_err());
    }
    foreign.validate(foreign.request()).unwrap();
    assert_eq!(port.control_word().unwrap(), 0x007f);
}

#[test]
fn issued_failed_com_history_can_reach_fpu_without_a_successful_live_apartment() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x003f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port).unwrap();
    let mut c = Construction::new(input(&mut host)).unwrap();
    let mut gate = gate(&mut host);
    gate.failed_com = true;
    for _ in 0..4096 {
        if matches!(
            c.pending_call(),
            Some(Call::ProviderVirtual {
                instruction: 0x4eee29
            })
        ) {
            break;
        }
        c.step(&mut gate).unwrap();
    }
    assert!(matches!(
        c.pending_call(),
        Some(Call::ProviderVirtual {
            instruction: 0x4eee29
        })
    ));
    assert_eq!(c.com_initialization().unwrap().hresult(), 0x80004005);
    assert!(c.com_initialization().unwrap().current_apartment().is_err());
    assert_eq!((gate.reads, gate.loads), (1, 0));
    drop(gate);
    assert_eq!(host.application_fpu_control(&c).unwrap(), 0x003f);
    c.validate().unwrap();
}

#[test]
fn null_wrapper_path_authenticates_com_history_without_inventing_a_mutex_return() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x003f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port).unwrap();
    let mut c = Construction::new(input(&mut host)).unwrap();
    let mut gate = gate(&mut host);
    gate.null_wrapper = true;
    for _ in 0..4096 {
        if matches!(
            c.pending_call(),
            Some(Call::ProviderVirtual {
                instruction: 0x4eee29
            })
        ) {
            break;
        }
        c.step(&mut gate).unwrap();
    }
    assert!(matches!(
        c.pending_call(),
        Some(Call::ProviderVirtual {
            instruction: 0x4eee29
        })
    ));
    assert!(c.mutex_return().is_none());
    assert!(c.mutex_wrapper().is_none());
    let read = c.fpu_read().unwrap();
    assert!(read.request().mutex_return().is_none());
    assert!(read.request().wrapper_image().is_none());
    assert_eq!(read.raw(), 0x003f);
    assert_eq!((gate.reads, gate.loads), (1, 0));
    drop(gate);
    assert_eq!(host.live_capabilities(), 3);
    assert_eq!(host.application_fpu_control(&c).unwrap(), 0x003f);
    c.validate().unwrap();
}

#[test]
fn genuine_api_null_mutex_history_remains_a_valid_fpu_source() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let port = install(0x003f, fpu::ExceptionState::Flags(0));
    host.configure_application_fpu(port).unwrap();
    let mut incoming = input(&mut host);
    let name = incoming
        .retained
        .iter_mut()
        .find(|o| o.data.starts_with(b"Global\\Sims\0"))
        .unwrap();
    name.data.fill(0);
    let invalid = b"Global\\invalid\\name\0";
    name.data[..invalid.len()].copy_from_slice(invalid);
    host.memory.synchronize_owner(name).unwrap();
    let mut c = Construction::new(incoming).unwrap();
    entry(&mut c, &mut host);
    pre_read(&mut c, &mut host);
    assert_eq!(c.mutex_return().unwrap().api_error(), Some(123));
    assert_eq!(c.mutex_return().unwrap().word(), 0);
    provider(&mut c, &mut host);
    assert_eq!(c.fpu_read().unwrap().raw(), 0x003f);
    assert_eq!(host.application_fpu_control(&c).unwrap(), 0x003f);
    assert_eq!(host.live_capabilities(), 4);
    c.validate().unwrap();
}
