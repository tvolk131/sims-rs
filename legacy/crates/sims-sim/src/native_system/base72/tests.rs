//! Authored against the parent-owned `native_system::base72` declaration.
//! UNRUN: this worker performs no compilation, tests, native calls or services.
use super::*;

fn incoming() -> Incoming {
    let mut caller_frame = Owner {
        token: 0x20000000,
        data: (0..96).map(|i| (i * 37 + 0x91) as u8).collect(),
    };
    let ebp = caller_frame.token + 48;
    caller_frame.data[56..60].fill(0);
    Incoming {
        parent: Owner {
            token: 0x10000000,
            data: (0..1240).map(|i| (i * 19 + 0x73) as u8).collect(),
        },
        caller_frame,
        ebp,
    }
}

#[derive(Default)]
struct TestHost {
    calls: usize,
    fail_first: bool,
    null_debug: bool,
    debug_token: Option<u64>,
    change: Option<fn(&mut InitializedSection)>,
    requests: Vec<Request>,
    last: Option<InitializedSection>,
}

impl Host for TestHost {
    fn initialize_section(&mut self, request: &Request) -> Result<InitializedSection, Stop> {
        request.validate()?;
        self.calls += 1;
        self.requests.push(request.clone());
        if self.fail_first {
            self.fail_first = false;
            return Err(Stop::Unavailable("fixture has no section reply yet".into()));
        }
        // A's tests exercise actual returned bytes through the existing layout
        // constructor. Resource/heap/lock authority belongs to B's real-host tests.
        let debug = (!self.null_debug).then(|| Owner {
            token: self.debug_token.unwrap_or(0x30000000), data: vec![0xda; 32],
        });
        let mut reply = InitializedSection::from_debug_info(&request.section_request(), debug.as_ref(), 7)?;
        if let Some(change) = self.change { change(&mut reply); }
        self.last = Some(reply.clone());
        Ok(reply)
    }
}

fn import_ready(input: Incoming) -> Construction {
    let mut cursor = Construction::new(input).unwrap();
    let mut host = TestHost::default();
    for _ in 0..11 {
        assert!(cursor.request().is_none());
        assert_eq!(cursor.step(&mut host).unwrap(), None);
        cursor.validate().unwrap();
    }
    assert_eq!(host.calls, 0);
    assert!(cursor.request().is_some());
    assert_eq!(cursor.writes().len(), 11);
    cursor
}

fn complete(mut cursor: Construction, host: &mut TestHost) -> Construction {
    for _ in 0..32 {
        if let Some(cut) = cursor.step(host).unwrap() {
            assert_eq!(cut, Cut::Base72Returned);
            assert!(cursor.returned());
            cursor.validate().unwrap();
            return cursor;
        }
    }
    panic!("Base72 failed to reach its bounded normal return");
}

fn put(owner: &mut Owner, offset: usize, value: u32) {
    owner.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn system_base72_exact_source_order_full_preimages_and_actual_section_merge() {
    let input = incoming();
    let mut cursor = import_ready(input.clone());
    let request = cursor.request().unwrap().clone();
    assert_eq!(request.initial_parent(), &input.parent);
    assert_eq!(request.initial_caller_frame(), &input.caller_frame);
    assert_eq!(request.ebp(), input.ebp);
    assert_eq!(request.parent(), cursor.parent());
    assert_eq!(request.caller_frame(), cursor.caller_frame());
    assert_eq!(request.section_request(), SectionRequest {
        instruction: 0x5d91bc, import_slot: 0x666114,
        region: Owner { token: input.parent.token + 60, data: vec![0; 24] },
    });
    let mut host = TestHost::default();
    assert_eq!(cursor.step(&mut host).unwrap(), None);
    assert_eq!(host.calls, 1);
    let returned = host.last.as_ref().unwrap().clone();
    assert_eq!(cursor.section(), Some(&returned));
    assert_eq!(cursor.parent(), request.parent());
    assert_eq!(cursor.caller_frame(), request.caller_frame());
    assert_eq!(cursor.writes().len(), 11); // Successful return is staged first.
    assert_eq!(cursor.step(&mut host).unwrap(), None);
    assert_eq!(&cursor.parent().data[60..84], returned.region.data.as_slice());
    let cursor = complete(cursor, &mut host);
    assert_eq!(host.calls, 1);
    assert_eq!(cursor.writes().iter().map(|w| w.instruction).collect::<Vec<_>>(), vec![
        0x5d5a2f, 0x5d5a32, 0x5d5a38, 0x5d5a3c, 0x5d5a43, 0x5d5a4a,
        0x5d5a51, 0x5d91a7, 0x5d91aa, 0x5d91ad, 0x5d91b3, 0x5d91bc,
        0x5d91c2, 0x5d9485, 0x5d9492, 0x5d5a62, 0x5d5a69, 0x5d5a6d,
        0x5d5a70, 0x5d5a76, 0x5d5a7c, 0x5d5a83,
    ]);
    let mut expected = input.parent.clone();
    for (member, value) in [
        (0, 0x673820), (4, 0x673814), (8, 0x6737fc), (12, 0),
        (16, 0x673838), (20, 0x673ab8), (48, 1), (52, 0), (56, 1), (60, 0), (68, 0),
    ] { put(&mut expected, 36 + member, value); }
    expected.data[60..84].copy_from_slice(&returned.region.data);
    expected.data[100] = 0;
    assert_eq!(cursor.parent(), &expected);
    let mut expected_frame = input.caller_frame.clone();
    put(&mut expected_frame, 32, (input.parent.token + 36) as u32);
    put(&mut expected_frame, 44, 0);
    assert_eq!(cursor.caller_frame(), &expected_frame);
    assert_eq!(&cursor.parent().data[..36], &input.parent.data[..36]);
    assert_eq!(&cursor.parent().data[108..], &input.parent.data[108..]);
    assert_eq!(&cursor.parent().data[101..104], &input.parent.data[101..104]);

    // Independently reverse every receipt to recover both COMPLETE preimages.
    let mut parent = cursor.parent().clone();
    let mut caller = cursor.caller_frame().clone();
    for receipt in cursor.writes().iter().rev() {
        let owner = if receipt.owner == parent.token { &mut parent } else {
            assert_eq!(receipt.owner, caller.token); &mut caller
        };
        let end = receipt.offset + receipt.after.len();
        assert_eq!(&owner.data[receipt.offset..end], receipt.after.as_slice());
        owner.data[receipt.offset..end].copy_from_slice(&receipt.before);
    }
    assert_eq!(parent, input.parent);
    assert_eq!(caller, input.caller_frame);
    assert!(cursor.request().unwrap().same_call(&request));
}

#[test]
fn system_base72_receipts_capture_overwrites_getter_and_byte_width() {
    let mut host = TestHost::default();
    let cursor = complete(Construction::new(incoming()).unwrap(), &mut host);
    let writes = cursor.writes();
    assert_eq!(writes.len(), 22);
    for (index, offset, after) in [
        (1, 36, 0x673840_u32), (3, 44, 0x673720), (4, 40, 0x673714),
        (5, 44, 0x6736fc), (12, 84, 1), (13, 52, 0x673858),
        (14, 92, 1), (17, 96, 0), (18, 52, 0x673838),
        (19, 36, 0x673820), (20, 40, 0x673814), (21, 44, 0x6737fc),
    ] {
        assert_eq!(writes[index].owner, cursor.parent().token);
        assert_eq!(writes[index].offset, offset);
        assert_eq!(writes[index].after, after.to_le_bytes());
    }
    assert_eq!(writes[5].before, 0x673720_u32.to_le_bytes());
    assert_eq!(writes[18].before, 0x673858_u32.to_le_bytes());
    assert_eq!(writes[19].before, 0x673840_u32.to_le_bytes());
    assert_eq!(writes[20].before, 0x673714_u32.to_le_bytes());
    assert_eq!(writes[21].before, 0x6736fc_u32.to_le_bytes());
    assert_eq!(writes[10].offset, 60);
    assert_eq!(writes[10].after, vec![0; 24]);
    assert_eq!(writes[11].before, vec![0; 24]);
    assert_eq!(writes[11].after, host.last.unwrap().region.data);
    assert_eq!(writes[15].offset, 100);
    assert_eq!(writes[15].after, vec![0]);
    for (index, write) in writes.iter().enumerate() {
        assert_eq!(write.child_return, matches!(index, 10 | 11));
        assert_eq!(write.before.len(), write.after.len());
    }
}

#[test]
fn system_base72_caller_dword_slots_preserve_words_padding_and_argument() {
    let input = incoming();
    assert_ne!(&input.caller_frame.data[46..48], &[0, 0]);
    let mut host = TestHost::default();
    let cursor = complete(Construction::new(input.clone()).unwrap(), &mut host);
    let caller = &cursor.caller_frame().data;
    assert_eq!(&caller[32..36], &(input.parent.token as u32 + 36).to_le_bytes());
    assert_eq!(&caller[44..48], &[0; 4]);
    assert_eq!(&caller[56..60], &[0; 4]);
    for index in 0..caller.len() {
        if !(32..36).contains(&index) && !(44..48).contains(&index) {
            assert_eq!(caller[index], input.caller_frame.data[index], "caller byte {index}");
        }
    }
    for value in [1_u32, 0x10000, 0x80000000, u32::MAX] {
        let mut wrong = input.clone();
        put(&mut wrong.caller_frame, 56, value);
        let error = Construction::new(wrong.clone()).unwrap_err();
        assert!(matches!(error.stop, Stop::Unsupported(_)));
        assert_eq!(error.incoming, wrong);
    }
}

#[test]
fn system_base72_admission_returns_original_owners_on_extent_overflow_and_alias() {
    let original = incoming();
    let mut cases = Vec::new();
    for length in [0, 72, 104, 324, 1239, 1241] {
        let mut input = original.clone(); input.parent.data.resize(length, 0x81); cases.push(input);
    }
    for token in [0, (1_u64 << 32) - 1239, 1_u64 << 32, u64::MAX - 8] {
        let mut input = original.clone(); input.parent.token = token; cases.push(input);
    }
    for token in [0, (1_u64 << 32) - 95, u64::MAX - 8] {
        let mut input = original.clone(); input.caller_frame.token = token; cases.push(input);
    }
    for ebp in [0, 15, original.caller_frame.token + 15, original.caller_frame.token + 85,
        (1_u64 << 32) - 11, u64::MAX - 1]
    {
        let mut input = original.clone(); input.ebp = ebp; cases.push(input);
    }
    let mut short = original.clone(); short.caller_frame.data.truncate(59); cases.push(short);
    let mut alias = original.clone();
    // Overlap the parent's far suffix, not merely the embedded section view.
    alias.caller_frame.token = alias.parent.token + 1200;
    alias.ebp = alias.caller_frame.token + 48;
    cases.push(alias);
    for input in cases {
        let error = Construction::new(input.clone()).unwrap_err();
        assert_eq!(error.incoming, input);
        assert!(matches!(error.stop, Stop::Unsupported(_)));
    }
    let mut actual = original;
    actual.parent.data.pop();
    let parent_storage = actual.parent.data.as_ptr();
    let caller_storage = actual.caller_frame.data.as_ptr();
    let error = Construction::new(actual).unwrap_err();
    // Failure returns the actual consumed owners, including their Vec storage,
    // rather than substituting fresh byte-equal allocations.
    assert_eq!(error.incoming.parent.data.as_ptr(), parent_storage);
    assert_eq!(error.incoming.caller_frame.data.as_ptr(), caller_storage);
}

#[test]
fn system_base72_exact_minimum_frame_and_native_end_exclusive_boundaries() {
    let mut input = incoming();
    input.caller_frame.data = (0..28).map(|i| (i * 17 + 0x31) as u8).collect();
    input.caller_frame.data[24..28].fill(0);
    input.ebp = input.caller_frame.token + 16;
    input.parent.token = (1_u64 << 32) - 1240;
    let mut host = TestHost::default();
    complete(Construction::new(input).unwrap(), &mut host).validate().unwrap();

    let mut input = incoming();
    input.caller_frame.token = (1_u64 << 32) - 96;
    input.ebp = (1_u64 << 32) - 12;
    input.caller_frame.data[92..96].fill(0);
    let mut host = TestHost { debug_token: Some((1_u64 << 32) - 128), ..TestHost::default() };
    complete(Construction::new(input).unwrap(), &mut host).validate().unwrap();
}

#[test]
fn system_base72_sealed_requests_bind_invocation_and_all_images() {
    let first = import_ready(incoming());
    let second = import_ready(incoming());
    let request = first.request().unwrap();
    let clone = request.clone();
    let other = second.request().unwrap();
    assert!(request.same_call(&clone));
    assert_eq!(request, &clone);
    assert_eq!(request.parent(), other.parent());
    assert_eq!(request.caller_frame(), other.caller_frame());
    assert!(!request.same_call(other));
    assert_ne!(request, other);
    for change in 0..6 {
        let mut edited = clone.clone();
        match change {
            0 => edited.parent.data[1200] ^= 1,
            1 => edited.caller_frame.data[1] ^= 1,
            2 => edited.initial_parent.data[1200] ^= 1,
            3 => edited.initial_caller_frame.data[1] ^= 1,
            4 => edited.ebp += 4,
            _ => edited.invocation = Arc::new(Invocation),
        }
        assert!(!request.same_call(&edited));
        if change != 5 { assert!(edited.validate().is_err()); }
    }
    request.validate().unwrap();
}

#[test]
fn system_base72_typed_partial_ownership_and_host_error_retry_keep_same_request() {
    let input = incoming();
    let initial = Construction::new(input.clone()).unwrap();
    let initial = initial.into_prefix().unwrap_err();
    assert_eq!(initial.parent(), &input.parent);
    assert_eq!(initial.caller_frame(), &input.caller_frame);
    let mut cursor = import_ready(input.clone());
    let request = cursor.request().unwrap().clone();
    let writes = cursor.writes().to_vec();
    let mut host = TestHost { fail_first: true, ..TestHost::default() };
    assert!(matches!(cursor.step(&mut host), Err(Stop::Unavailable(_))));
    cursor.validate().unwrap();
    assert_eq!(cursor.parent(), request.parent());
    assert_eq!(cursor.caller_frame(), request.caller_frame());
    assert_eq!(cursor.writes(), writes);
    assert!(cursor.section().is_none());
    assert_eq!(cursor.incoming, input);
    let mut cursor = cursor.into_prefix().unwrap_err();
    assert_eq!(cursor.step(&mut host).unwrap(), None);
    assert_eq!(host.calls, 2);
    assert!(host.requests[0].same_call(&host.requests[1]));
    assert!(cursor.request().unwrap().same_call(&request));
    let cursor = complete(cursor, &mut host);
    assert_eq!(host.calls, 2);
    cursor.into_prefix().unwrap().validate().unwrap();
}

#[test]
fn system_base72_staged_malformed_returns_remain_owned_and_never_reissue() {
    let changes: [fn(&mut InitializedSection); 10] = [
        |r| r.capability = 0,
        |r| r.region.token += 4,
        |r| { r.region.data.pop(); },
        |r| r.region.data[4] = 0,
        |r| r.debug_info = None,
        |r| r.debug_info.as_mut().unwrap().data[4] ^= 1,
        |r| { r.debug_info.as_mut().unwrap().data.pop(); },
        |r| r.debug_info.as_mut().unwrap().token = 0,
        |r| r.debug_info.as_mut().unwrap().token = (1_u64 << 32) - 31,
        |r| r.debug_info.as_mut().unwrap().token = u64::MAX - 4,
    ];
    for change in changes {
        let mut cursor = import_ready(incoming());
        let request = cursor.request().unwrap().clone();
        let writes = cursor.writes().to_vec();
        let mut host = TestHost { change: Some(change), ..TestHost::default() };
        assert!(matches!(cursor.step(&mut host), Err(Stop::Unsupported(_))));
        let actual = host.last.as_ref().unwrap().clone();
        assert_eq!(cursor.section(), Some(&actual));
        assert_eq!(cursor.parent(), request.parent());
        assert_eq!(cursor.caller_frame(), request.caller_frame());
        assert_eq!(cursor.writes(), writes);
        assert!(cursor.validate().is_err());
        // A host offering a now-valid response cannot replace the retained
        // invalid result, merge it, or perform a second initialization.
        host.change = None;
        for _ in 0..3 { assert!(cursor.step(&mut host).is_err()); }
        assert_eq!(host.calls, 1);
        assert_eq!(cursor.section(), Some(&actual));
        let cursor = cursor.into_prefix().unwrap_err();
        assert_eq!(cursor.section(), Some(&actual));
        assert!(!cursor.returned());
    }
}

#[test]
fn system_base72_debug_alias_checks_use_complete_parent_and_caller_owners() {
    let input = incoming();
    for token in [input.parent.token + 200, input.caller_frame.token] {
        let mut cursor = import_ready(input.clone());
        let request = cursor.request().unwrap().clone();
        let mut host = TestHost { debug_token: Some(token), ..TestHost::default() };
        assert!(cursor.step(&mut host).is_err());
        let reply = cursor.section().unwrap();
        // The preexisting section-only validator accepts these layouts; the
        // full containing ownership checks are the additional rejection.
        reply.validate(&request.section_request()).unwrap();
        assert_eq!(reply.debug_info.as_ref().unwrap().token, token);
        assert!(cursor.step(&mut host).is_err());
        assert_eq!(host.calls, 1);
        assert_eq!(cursor.parent(), request.parent());
    }
}

#[test]
fn system_base72_genuine_null_debug_return_retains_capability_and_exact_bytes() {
    let mut cursor = import_ready(incoming());
    let mut host = TestHost { null_debug: true, ..TestHost::default() };
    cursor.step(&mut host).unwrap();
    let staged = cursor.section().unwrap();
    assert_eq!(staged.capability, 7);
    assert!(staged.debug_info.is_none());
    let mut expected = vec![0; 24]; expected[4..8].fill(0xff);
    assert_eq!(staged.region.data, expected);
    let cursor = complete(cursor, &mut host);
    assert_eq!(host.calls, 1);
    let prefix = cursor.into_prefix().unwrap();
    prefix.validate().unwrap();
    assert!(prefix.section().debug_info.is_none());
    assert_eq!(prefix.section().region.data, expected);
}

#[test]
fn system_base72_valid_staged_return_merge_and_completed_replay_make_one_call() {
    let mut cursor = import_ready(incoming());
    let history_capacity = cursor.history.capacity();
    let buffers = cursor.history.iter().map(|r| (r.before.as_ptr(), r.after.as_ptr())).collect::<Vec<_>>();
    let mut host = TestHost::default();
    cursor.step(&mut host).unwrap();
    let actual = cursor.section().unwrap().clone();
    assert_eq!(cursor.writes().len(), 11);
    assert!(!cursor.returned());
    let mut cursor = complete(cursor, &mut host);
    let parent = cursor.parent().clone();
    let caller = cursor.caller_frame().clone();
    let writes = cursor.writes().to_vec();
    for _ in 0..3 {
        assert_eq!(cursor.step(&mut host).unwrap(), Some(Cut::Base72Returned));
        cursor.validate().unwrap();
    }
    assert_eq!(host.calls, 1);
    assert_eq!(cursor.parent(), &parent);
    assert_eq!(cursor.caller_frame(), &caller);
    assert_eq!(cursor.writes(), writes);
    assert_eq!(cursor.section(), Some(&actual));
    assert_eq!(cursor.history.capacity(), history_capacity);
    assert_eq!(cursor.history.iter().map(|r| (r.before.as_ptr(), r.after.as_ptr())).collect::<Vec<_>>(), buffers);
    let prefix = cursor.into_prefix().unwrap();
    for _ in 0..3 { prefix.validate().unwrap(); }
    assert_eq!(prefix.parent(), &parent);
    assert_eq!(prefix.caller_frame(), &caller);
    assert_eq!(prefix.section(), &actual);
    assert_eq!(prefix.writes(), writes);
    assert!(prefix.request().same_call(&host.requests[0]));
    assert_eq!(host.calls, 1);
}

#[test]
fn system_base72_validation_rejects_source_history_and_full_image_tampering() {
    for change in 0..15 {
        let mut cursor = import_ready(incoming());
        match change {
            0 => cursor.parent.data[1200] ^= 1,
            1 => cursor.caller_frame.data[1] ^= 1,
            2 => cursor.history[5].before[0] ^= 1,
            3 => cursor.history[5].after[0] ^= 1,
            4 => cursor.history[5].instruction ^= 1,
            5 => cursor.history[5].offset += 1,
            6 => cursor.history[5].owner += 1,
            7 => cursor.history[5].child_return = true,
            8 => { cursor.history[5].before.pop(); }
            9 => cursor.history[11].after[0] = 1,
            10 => cursor.next -= 1,
            11 => cursor.sealed = false,
            12 => cursor.invocation = Arc::new(Invocation),
            13 => cursor.request.parent.data[1200] ^= 1,
            _ => cursor.request.initial_caller_frame.data[1] ^= 1,
        }
        assert!(cursor.validate().is_err(), "tamper {change}");
        let mut host = TestHost::default();
        assert!(cursor.step(&mut host).is_err());
        assert_eq!(host.calls, 0);
        assert!(cursor.into_prefix().is_err());
    }
}

#[test]
fn system_base72_staged_reply_survives_source_stop_without_reset_or_reissue() {
    let mut cursor = import_ready(incoming());
    let mut host = TestHost::default();
    cursor.step(&mut host).unwrap();
    let staged = cursor.section().unwrap().clone();
    cursor.caller_frame.data[1] ^= 1;
    assert!(cursor.step(&mut host).is_err());
    assert_eq!(cursor.section(), Some(&staged));
    assert_eq!(host.calls, 1);
    // Only test-private restoration of the original byte permits resumption;
    // the public API has no setter or replacement-return escape hatch.
    cursor.caller_frame.data[1] ^= 1;
    let cursor = complete(cursor, &mut host);
    assert_eq!(cursor.section(), Some(&staged));
    assert_eq!(host.calls, 1);
}

#[test]
fn system_base72_completed_prefix_revalidates_images_return_and_identity() {
    for change in 0..9 {
        let mut host = TestHost::default();
        let mut prefix = complete(Construction::new(incoming()).unwrap(), &mut host).into_prefix().unwrap();
        match change {
            0 => prefix.construction.parent.data[1200] ^= 1,
            1 => prefix.construction.caller_frame.data[1] ^= 1,
            2 => prefix.construction.section.as_mut().unwrap().capability = 0,
            3 => prefix.construction.section.as_mut().unwrap().debug_info.as_mut().unwrap().data[4] ^= 1,
            4 => prefix.construction.history[14].after[0] = 2,
            5 => prefix.construction.request.invocation = Arc::new(Invocation),
            6 => prefix.construction.section.as_mut().unwrap().capability = 8,
            7 => prefix.construction.reply_receipt.as_mut().unwrap().capability = 8,
            _ => prefix.construction.next = RETURN,
        }
        assert!(prefix.validate().is_err(), "prefix tamper {change}");
        assert_eq!(host.calls, 1);
    }
}

#[test]
fn system_base72_only_explicit_normal_child_return_can_form_prefix() {
    let mut cursor = import_ready(incoming());
    let mut host = TestHost::default();
    while cursor.writes().len() < 22 {
        assert_eq!(cursor.step(&mut host).unwrap(), None);
        assert!(!cursor.returned());
    }
    let mut cursor = cursor.into_prefix().unwrap_err();
    assert_eq!(cursor.step(&mut host).unwrap(), Some(Cut::Base72Returned));
    let prefix = cursor.into_prefix().unwrap();
    prefix.validate().unwrap();
    assert_eq!(prefix.parent().data.len(), 1240);
    assert_eq!(prefix.writes().len(), 22);
    assert_eq!(prefix.writes().last().unwrap().instruction, 0x5d5a83);
    assert!(prefix.writes().iter().filter(|w| w.owner == prefix.parent().token)
        .all(|w| w.offset >= 36 && w.offset + w.after.len() <= 108));
    // This cut and opaque prefix certify only the stated Base72 child effects;
    // there is no provider/global/Application/Game/World completion producer.
    assert_eq!(host.calls, 1);
}
