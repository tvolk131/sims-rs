use super::*;

const APPLICATION: u64 = 0x10000000;

fn source() -> ReadRequest {
    let mut application = Owner {
        token: APPLICATION,
        data: (0..324).map(|i| (i % 251) as u8).collect(),
    };
    application.data[248] = 0;
    // The source ignores HRESULT. A genuine declared failed COM history is
    // sufficient; these tests neither initialize native COM nor send IPC.
    let com = com::Return::failed(&com::Request::new(APPLICATION), 0x80004005).unwrap();
    ReadRequest::new(
        &Invocation::new(APPLICATION),
        &application,
        &com,
        None,
        None,
    )
}

fn install(control_word: u16, exceptions: ExceptionState) -> Port {
    Port::install(InitialState {
        control_word,
        exceptions,
    })
    .unwrap()
}

fn unavailable<T: std::fmt::Debug>(result: Result<T, Stop>) {
    assert!(matches!(result, Err(Stop::Unavailable(_))), "{result:?}");
}

#[test]
fn sealed_source_owns_images_and_distinguishes_invocation_and_com_history() {
    let request = source();
    request.validate().unwrap();
    let mut application = request.application_image().clone();
    let invocation = Invocation::new(APPLICATION);
    let bound = ReadRequest::new(&invocation, &application, request.com_return(), None, None);
    let same = ReadRequest::new(&invocation, &application, request.com_return(), None, None);
    assert_eq!(bound, same);
    application.data[17] ^= 1;
    assert_ne!(bound.application_image(), &application);
    assert_eq!(bound.com_return(), request.com_return());
    assert!(bound.mutex_return().is_none());
    assert!(bound.wrapper_image().is_none());

    let other_invocation = ReadRequest::new(
        &Invocation::new(APPLICATION),
        bound.application_image(),
        bound.com_return(),
        None,
        None,
    );
    let other_com = com::Return::failed(&com::Request::new(APPLICATION), 0x80004005).unwrap();
    let other_history = ReadRequest::new(
        &invocation,
        bound.application_image(),
        &other_com,
        None,
        None,
    );
    other_invocation.validate().unwrap();
    other_history.validate().unwrap();
    let port = install(0x003f, ExceptionState::Flags(0));
    let read = port.read_control(&bound).unwrap();
    read.validate(&same).unwrap();
    assert!(read.validate(&other_invocation).is_err());
    assert!(read.validate(&other_history).is_err());
    assert_eq!(read.serial(), 1);
}

#[test]
fn malformed_source_stops_before_any_read_operation() {
    let request = source();
    let port = install(0x037f, ExceptionState::Flags(0));
    let mut bad = Vec::new();
    let mut candidate = request.clone();
    candidate.application.data.pop();
    bad.push(candidate);
    let mut candidate = request.clone();
    candidate.application.data.push(0);
    bad.push(candidate);
    let mut candidate = request.clone();
    candidate.application.token = 0;
    bad.push(candidate);
    let mut candidate = request.clone();
    candidate.application.token = (1_u64 << 32) - 323;
    bad.push(candidate);
    let mut candidate = request.clone();
    candidate.invocation = Invocation::new(APPLICATION + 4);
    bad.push(candidate);
    let mut candidate = request.clone();
    candidate.com = com::Return::failed(&com::Request::new(APPLICATION + 4), 0x80004005).unwrap();
    bad.push(candidate);
    let mut candidate = request.clone();
    candidate.application.data[248] = 1;
    bad.push(candidate);
    let mut candidate = request.clone();
    candidate.wrapper = Some(Owner {
        token: 0x20000000,
        data: vec![0; 12],
    });
    bad.push(candidate);
    for candidate in bad {
        assert!(matches!(
            port.read_control(&candidate),
            Err(Stop::Unsupported(_))
        ));
        assert_eq!(port.control_word().unwrap(), 0x037f);
        assert_eq!(port.0.state.borrow().serial, 0);
    }
    assert_eq!(port.read_control(&request).unwrap().serial(), 1);
}

#[test]
fn initial_state_is_explicit_and_invalid_flag_bits_do_not_install() {
    assert!(CURRENT.with(|slot| slot.borrow().upgrade().is_none()));
    assert!(matches!(
        Port::install(InitialState {
            control_word: 0x003f,
            exceptions: ExceptionState::Flags(0x40),
        }),
        Err(Stop::Unsupported(_))
    ));
    assert!(CURRENT.with(|slot| slot.borrow().upgrade().is_none()));
    let port = install(0xabcd, ExceptionState::Flags(0));
    assert_eq!(port.control_word().unwrap(), 0xabcd);
    assert_eq!(port.read_control(&source()).unwrap().raw(), 0xabcd);
}

#[test]
fn separate_handles_share_state_and_never_reinitialize_a_live_context() {
    let first = install(0x037f, ExceptionState::Flags(0));
    let second = first.clone();
    let request = source();
    let read = first.read_control(&request).unwrap();
    unavailable(Port::install(InitialState {
        control_word: 0x003f,
        exceptions: ExceptionState::Unknown,
    }));
    assert!(second.owns_read(&read));
    let loaded = second.load_control(&LoadRequest::new(&read)).unwrap();
    assert_eq!((loaded.before(), loaded.after()), (0x037f, 0x007f));
    assert_eq!(first.control_word().unwrap(), 0x007f);
    assert_eq!(read.current_word().unwrap(), 0x007f);
    assert_eq!(read.raw(), 0x037f);
}

#[test]
fn same_request_direct_calls_are_distinct_operations_and_clones_share_history() {
    let port = install(0x003f, ExceptionState::Flags(0));
    let request = source();
    let first = port.read_control(&request).unwrap();
    let clone = first.clone();
    let second = port.read_control(&request).unwrap();
    assert_eq!(first, clone);
    assert_ne!(first, second);
    assert_eq!((first.raw(), second.raw()), (0x003f, 0x003f));
    assert_eq!((first.serial(), second.serial()), (1, 2));
    drop(first);
    clone.validate(&request).unwrap();
    assert!(port.owns_read(&clone));
}

#[test]
fn unchanged_reads_cannot_authorize_a_load() {
    for word in [0x003f, 0x007f, 0xf0ff] {
        let port = install(word, ExceptionState::Flags(0x3f));
        let read = port.read_control(&source()).unwrap();
        assert!(!changed(read.raw()));
        let request = LoadRequest::new(&read);
        assert!(request.validate().is_err());
        assert!(port.load_control(&request).is_err());
        assert_eq!(port.control_word().unwrap(), word);
        assert_eq!(port.0.state.borrow().serial, 1);
    }
}

#[test]
fn changed_precision_masks_and_rounding_use_owned_load_operations() {
    // Expected operands are fixed examples, not another copy of the formula.
    for (before, after) in [
        (0x037f, 0x007f),
        (0x003e, 0x003f),
        (0x043f, 0x003f),
        (0x083f, 0x003f),
        (0x0c3f, 0x003f),
        (0xffff, 0xf0ff),
    ] {
        let port = install(before, ExceptionState::Flags(0));
        let source = source();
        let read = port.read_control(&source).unwrap();
        assert!(changed(read.raw()));
        let request = LoadRequest::new(&read);
        request.validate().unwrap();
        assert_eq!(request.read(), &read);
        assert_eq!(request.control(), after);
        assert_eq!(normalized(read.raw()), after);
        let loaded = port.load_control(&request).unwrap();
        loaded.validate(&request).unwrap();
        assert_eq!(loaded.request(), &request);
        assert_eq!(
            (loaded.before(), loaded.after(), loaded.serial()),
            (before, after, 2)
        );
        assert_eq!(port.control_word().unwrap(), after);
        assert_eq!(read.current_word().unwrap(), after);
        read.validate(&source).unwrap();
    }
}

#[test]
fn later_load_uses_original_operand_and_actual_current_before_word() {
    let port = install(0x037f, ExceptionState::Flags(0x21));
    let source = source();
    let read = port.read_control(&source).unwrap();
    let request = LoadRequest::new(&read);
    let first = port.load_control(&request).unwrap();
    let second = port.load_control(&request).unwrap();
    assert_eq!((first.before(), second.before()), (0x037f, 0x007f));
    assert_eq!((first.after(), second.after()), (0x007f, 0x007f));
    assert_eq!((first.serial(), second.serial()), (2, 3));
    assert_ne!(first, second);
    assert_eq!(first, first.clone());
    assert_eq!(
        port.0.state.borrow().exceptions,
        ExceptionState::Flags(0x21)
    );
    let now = port.read_control(&source).unwrap();
    assert_eq!((now.raw(), now.serial()), (0x007f, 4));
    assert!(LoadRequest::new(&now).validate().is_err());
    read.validate(&source).unwrap();
    first.validate(&request).unwrap();
    second.validate(&request).unwrap();
}

#[test]
fn each_masked_pending_flag_survives_read_and_normalized_load() {
    for bit in 0..6 {
        let flags = 1_u8 << bit;
        let port = install(0x037f, ExceptionState::Flags(flags));
        let read = port.read_control(&source()).unwrap();
        let loaded = port.load_control(&LoadRequest::new(&read)).unwrap();
        assert_eq!(loaded.after(), 0x007f);
        assert_eq!(
            port.0.state.borrow().exceptions,
            ExceptionState::Flags(flags)
        );
        assert_eq!(port.read_control(read.request()).unwrap().serial(), 3);
    }
}

#[test]
fn each_known_pending_unmasked_flag_stops_before_fnstcw() {
    for bit in 0..6 {
        let control = 0x037f & !(1_u16 << bit);
        let port = install(control, ExceptionState::Flags(1_u8 << bit));
        unavailable(port.read_control(&source()));
        assert_eq!(port.control_word().unwrap(), control);
        assert_eq!(port.0.state.borrow().serial, 0);
    }
}

#[test]
fn unmasked_but_not_pending_is_readable_and_unknown_state_is_unavailable() {
    {
        let port = install(0, ExceptionState::Flags(0));
        let read = port.read_control(&source()).unwrap();
        assert_eq!(read.raw(), 0);
        assert_eq!(
            port.load_control(&LoadRequest::new(&read)).unwrap().after(),
            0x003f
        );
    }
    let port = install(0x003f, ExceptionState::Unknown);
    unavailable(port.read_control(&source()));
    assert_eq!(port.control_word().unwrap(), 0x003f);
    assert_eq!(port.0.state.borrow().serial, 0);
}

#[test]
fn load_checks_preexisting_wait_state_before_masking_or_committing() {
    let port = install(0x033e, ExceptionState::Flags(0));
    let read = port.read_control(&source()).unwrap();
    let request = LoadRequest::new(&read);
    // Private fault injection exercises the load guard independently of the
    // read guard. There is deliberately no public flag mutation API.
    for exceptions in [ExceptionState::Unknown, ExceptionState::Flags(1)] {
        port.0.state.borrow_mut().exceptions = exceptions;
        unavailable(port.load_control(&request));
        assert_eq!(port.control_word().unwrap(), 0x033e);
        let state = port.0.state.borrow();
        assert_eq!((state.serial, state.exceptions), (1, exceptions));
        read.validate(read.request()).unwrap();
    }
    port.0.state.borrow_mut().exceptions = ExceptionState::Flags(0);
    let loaded = port.load_control(&request).unwrap();
    assert_eq!(
        (loaded.before(), loaded.after(), loaded.serial()),
        (0x033e, 0x003f, 2)
    );
}

#[test]
fn retirement_invalidates_every_handle_but_preserves_historical_validation() {
    let port = install(0x037f, ExceptionState::Flags(0x3f));
    let clone = port.clone();
    let source = source();
    let read = port.read_control(&source).unwrap();
    let request = LoadRequest::new(&read);
    let loaded = clone.load_control(&request).unwrap();
    port.retire().unwrap();
    unavailable(port.check_live());
    unavailable(clone.control_word());
    unavailable(read.current_word());
    unavailable(clone.read_control(&source));
    unavailable(clone.load_control(&request));
    unavailable(port.retire());
    assert!(clone.owns_read(&read));
    read.validate(&source).unwrap();
    loaded.validate(&request).unwrap();
    assert_eq!(
        (read.raw(), loaded.before(), loaded.after()),
        (0x037f, 0x037f, 0x007f)
    );
}

#[test]
fn last_actual_owner_drop_retires_even_when_all_receipts_are_retained() {
    let port = install(0x037f, ExceptionState::Flags(0x20));
    let clone = port.clone();
    let source = source();
    let read = port.read_control(&source).unwrap();
    let read_clone = read.clone();
    let request = LoadRequest::new(&read);
    let loaded = port.load_control(&request).unwrap();
    let loaded_clone = loaded.clone();
    let context = Rc::downgrade(&port.0);
    drop(port);
    clone.check_live().unwrap();
    assert_eq!(read_clone.current_word().unwrap(), 0x007f);
    drop(clone);
    assert!(context.upgrade().is_none());
    unavailable(read.current_word());
    read_clone.validate(&source).unwrap();
    loaded_clone.validate(&request).unwrap();

    let fresh = install(0x1234, ExceptionState::Flags(0));
    assert_eq!(fresh.control_word().unwrap(), 0x1234);
    assert!(!fresh.owns_read(&read));
    assert!(fresh.load_control(&request).is_err());
    assert_eq!(fresh.0.state.borrow().serial, 0);
    assert_eq!(fresh.read_control(&source).unwrap().raw(), 0x1234);
}

#[test]
fn replacement_epoch_never_adopts_old_receipts_or_old_handle_cleanup() {
    let old = install(0x037f, ExceptionState::Flags(0));
    let old_clone = old.clone();
    let source = source();
    let read = old.read_control(&source).unwrap();
    let request = LoadRequest::new(&read);
    old.retire().unwrap();
    let fresh = install(0x037f, ExceptionState::Flags(0));
    let current = fresh.read_control(&source).unwrap();
    assert_eq!(
        (current.raw(), current.serial()),
        (read.raw(), read.serial())
    );
    assert_ne!(current, read);
    assert!(!fresh.owns_read(&read));
    assert!(!old.owns_read(&current));
    assert!(fresh.load_control(&request).is_err());
    unavailable(old_clone.read_control(&source));
    drop(old);
    drop(old_clone);
    fresh.check_live().unwrap();
    assert_eq!(current.current_word().unwrap(), 0x037f);
    unavailable(read.current_word());
    read.validate(&source).unwrap();
    assert_eq!(fresh.0.state.borrow().serial, 1);
}

#[test]
fn different_threads_install_independent_authority_while_main_context_is_live() {
    let port = install(0x037f, ExceptionState::Flags(0));
    let read = port.read_control(&source()).unwrap();
    let other = std::thread::spawn(|| {
        let port = install(0x083f, ExceptionState::Flags(0x3f));
        let read = port.read_control(&source()).unwrap();
        let loaded = port.load_control(&LoadRequest::new(&read)).unwrap();
        port.retire().unwrap();
        read.validate(read.request()).unwrap();
        loaded.validate(loaded.request()).unwrap();
        (read.raw(), loaded.after(), loaded.serial())
    })
    .join()
    .unwrap();
    assert_eq!(other, (0x083f, 0x003f, 2));
    assert!(port.owns_read(&read));
    assert_eq!(port.control_word().unwrap(), 0x037f);
    assert_eq!(port.0.state.borrow().serial, 1);
}

#[test]
fn quota_and_serial_overflow_fail_before_read_or_load_effects() {
    let port = install(0x037f, ExceptionState::Flags(0x3f));
    port.0.state.borrow_mut().serial = MAX_OPERATIONS - 2;
    let source = source();
    let read = port.read_control(&source).unwrap();
    assert_eq!(read.serial(), MAX_OPERATIONS - 1);
    let request = LoadRequest::new(&read);
    let loaded = port.load_control(&request).unwrap();
    assert_eq!(loaded.serial(), MAX_OPERATIONS);
    for serial in [MAX_OPERATIONS, u64::MAX] {
        port.0.state.borrow_mut().serial = serial;
        unavailable(port.read_control(&source));
        unavailable(port.load_control(&request));
        let state = port.0.state.borrow();
        assert_eq!(state.serial, serial);
        assert_eq!(state.control_word, 0x007f);
        assert_eq!(state.exceptions, ExceptionState::Flags(0x3f));
    }
    loaded.validate(&request).unwrap();
    read.validate(&source).unwrap();
}

#[test]
fn borrowed_state_fails_without_committing_and_history_needs_no_live_borrow() {
    let port = install(0x037f, ExceptionState::Flags(0));
    let source = source();
    let read = port.read_control(&source).unwrap();
    let request = LoadRequest::new(&read);
    let state = port.0.state.borrow_mut();
    unavailable(port.read_control(&source));
    unavailable(port.load_control(&request));
    unavailable(port.retire());
    unavailable(read.current_word());
    assert!(port.owns_read(&read));
    read.validate(&source).unwrap();
    assert_eq!((state.serial, state.control_word), (1, 0x037f));
    drop(state);
    assert_eq!(port.load_control(&request).unwrap().serial(), 2);
}

#[test]
fn malformed_load_operand_and_foreign_read_identity_fail_before_mutation() {
    let port = install(0x037f, ExceptionState::Flags(0));
    let source = source();
    let first = port.read_control(&source).unwrap();
    let second = port.read_control(&source).unwrap();
    let valid = LoadRequest::new(&first);
    let other_read = LoadRequest::new(&second);
    let mut wrong_operand = valid.clone();
    wrong_operand.control ^= 1;
    assert!(port.load_control(&wrong_operand).is_err());
    assert_eq!(
        (port.0.state.borrow().serial, port.control_word().unwrap()),
        (2, 0x037f)
    );
    let loaded = port.load_control(&valid).unwrap();
    assert!(loaded.validate(&other_read).is_err());
    loaded.validate(&valid).unwrap();
    assert_eq!(loaded.serial(), 3);
}

#[test]
fn historical_validators_reject_corrupted_private_receipt_evidence() {
    let port = install(0x037f, ExceptionState::Flags(0));
    let source = source();
    let mut read = port.read_control(&source).unwrap();
    Rc::get_mut(&mut read.0).unwrap().serial = 0;
    assert!(read.validate(&source).is_err());

    let read = port.read_control(&source).unwrap();
    let request = LoadRequest::new(&read);
    let mut loaded = port.load_control(&request).unwrap();
    Rc::get_mut(&mut loaded.0).unwrap().after ^= 1;
    assert!(loaded.validate(&request).is_err());
    assert_eq!(port.control_word().unwrap(), 0x007f);
    assert_eq!(port.0.state.borrow().serial, 3);
}
