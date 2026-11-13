use super::*;

#[derive(Default)]
struct Services {
    calls: Vec<Request>,
    frees: Vec<u64>,
    reply: Option<Allocated>,
    deny_allocate: bool,
    deny_free: bool,
}

impl Host for Services {
    fn allocate_string(&mut self, request: Request) -> Result<Allocated, Stop> {
        if self.deny_allocate {
            return Err(Stop::Unavailable("allocator unavailable".into()));
        }
        self.calls.push(request);
        Ok(self.reply.take().unwrap_or_else(|| Allocated::Bytes {
            token: 0x400000 + self.calls.len() as u64 * 0x1000,
            data: vec![0xa5; request.bytes() as usize],
        }))
    }

    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        if self.deny_free {
            return Err(Stop::Unavailable("release unavailable".into()));
        }
        self.frees.push(token);
        Ok(())
    }
}

fn construction(input: Option<Vec<u8>>) -> Construction {
    Construction::new(
        Owner {
            token: 0x200000,
            data: vec![0x3c; 12],
        },
        input,
    )
    .unwrap()
}

fn finish(mut execution: Construction, host: &mut Services) -> State {
    for _ in 0..20 {
        let complete = execution.step(host).unwrap();
        let restored = serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(execution, restored);
        execution = restored;
        if complete {
            return execution.into_state().unwrap();
        }
    }
    panic!("string constructor did not return");
}

fn edit(mut execution: Edit, host: &mut Services) -> State {
    for _ in 0..10 {
        let complete = execution.step(host).unwrap();
        let restored = serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(execution, restored);
        execution = restored;
        if complete {
            return execution.into_state().unwrap();
        }
    }
    panic!("string edit did not return");
}

#[test]
fn constructors_distinguish_default_reserve_from_exact_empty_or_binary_copy() {
    for (input, requested, expected) in [
        (None, 8, vec![]),
        (Some(vec![]), 1, vec![]),
        (Some(b"./".to_vec()), 3, b"./".to_vec()),
        (Some(vec![65, 0, 66]), 4, vec![65, 0, 66]),
    ] {
        let mut host = Services::default();
        let state = finish(construction(input), &mut host);
        assert_eq!(host.calls, [Request::Construct(requested)]);
        assert!(host.frees.is_empty());
        assert_eq!(state.bytes().unwrap(), expected);
        assert!(
            state.backing.data[expected.len() + 1..]
                .iter()
                .all(|byte| *byte == 0xa5)
        );
    }
}

#[test]
fn edits_preserve_native_growth_old_prefix_terminator_and_unwritten_capacity() {
    let mut host = Services::default();
    let state = finish(construction(Some(b"abcdef".to_vec())), &mut host);
    let old = state.backing.token;
    let state = edit(
        Edit::new(state, Operation::Assign(b"ZYXWVUT".to_vec())).unwrap(),
        &mut host,
    );
    assert_eq!(state.backing.data.len(), 13);
    assert_eq!(state.bytes().unwrap(), b"ZYXWVUT");
    assert_eq!(&state.backing.data[8..], &[0xa5; 5]);
    assert_eq!(host.calls, [Request::Construct(7), Request::Grow(13)]);
    assert_eq!(host.frees, [old]);
    let state = edit(
        Edit::new(state, Operation::Assign(b"hi".to_vec())).unwrap(),
        &mut host,
    );
    assert_eq!(state.bytes().unwrap(), b"hi");
    assert_eq!(&state.backing.data[..8], b"hi\0WVUT\0");
    let token = state.backing.token;
    let state = edit(
        Edit::new(state, Operation::Append(b"!".to_vec())).unwrap(),
        &mut host,
    );
    assert_eq!(state.bytes().unwrap(), b"hi!");
    assert_eq!(state.backing.token, token);
    assert_eq!(host.calls.len(), 2);
}

#[test]
fn normalization_matches_original_slash_and_c_locale_ascii_rules_without_allocation() {
    let mut host = Services::default();
    let state = finish(
        construction(Some(b"./GameData/LIGHTMAP.DAT".to_vec())),
        &mut host,
    );
    let token = state.backing.token;
    let state = edit(
        Edit::new(state, Operation::CanonicalizeAscii).unwrap(),
        &mut host,
    );
    assert_eq!(state.bytes().unwrap(), b".\\gamedata\\lightmap.dat");
    assert_eq!(state.backing.token, token);
    assert_eq!(host.calls.len(), 1);
    assert!(host.frees.is_empty());
    let state = finish(construction(Some(vec![0xe9])), &mut host);
    assert!(Edit::new(state, Operation::CanonicalizeAscii).is_err());
}

#[test]
fn unanswered_allocation_retries_but_invalid_received_responses_stay_owned() {
    let mut host = Services {
        deny_allocate: true,
        ..Services::default()
    };
    let mut execution = construction(None);
    for _ in 0..3 {
        execution.step(&mut host).unwrap();
    }
    let before = execution.clone();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, before);
    host.deny_allocate = false;
    for response in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x200004,
            data: vec![0; 8],
        },
        Allocated::Bytes {
            token: 0xfffffff8,
            data: vec![0; 8],
        },
        Allocated::Bytes {
            token: 0x900000,
            data: vec![0; 7],
        },
    ] {
        let mut branch = execution.clone();
        host.reply = Some(response);
        branch.step(&mut host).unwrap();
        let before = branch.clone();
        let count = host.calls.len();
        for _ in 0..2 {
            assert!(branch.step(&mut host).is_err());
            assert_eq!(branch, before);
            assert_eq!(host.calls.len(), count);
        }
    }
}

#[test]
fn growth_keeps_both_allocations_until_successful_original_free_and_rejects_aliases() {
    let mut host = Services::default();
    let state = finish(construction(Some(b"abc".to_vec())), &mut host);
    let mut execution = Edit::new(state.clone(), Operation::Append(b"def".to_vec())).unwrap();
    for _ in 0..3 {
        execution.step(&mut host).unwrap();
    }
    let pending = execution.clone();
    let encoded = serde_json::to_value(&pending).unwrap();
    let mut changed = encoded.clone();
    changed["pending"] =
        serde_json::json!({"Bytes":{"token":state.backing.token,"data":vec![0;7]}});
    let mut aliased: Edit = serde_json::from_value(changed).unwrap();
    assert!(aliased.step(&mut host).is_err());
    assert!(host.frees.is_empty());
    execution.step(&mut host).unwrap();
    assert!(execution.replacement().is_some());
    assert_eq!(execution.state(), &state);
    host.deny_free = true;
    let before = execution.clone();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, before);
    host.deny_free = false;
    assert!(execution.step(&mut host).unwrap());
    assert_eq!(host.frees, [state.backing.token]);
    assert!(execution.step(&mut host).unwrap());
    assert_eq!(host.frees.len(), 1);
}

#[test]
fn corrupted_metadata_phases_and_complete_payloads_fail_closed() {
    let mut host = Services::default();
    let state = finish(construction(None), &mut host);
    let mut execution =
        Edit::new(state, Operation::Assign(b"GameData/lightmap.dat".to_vec())).unwrap();
    while !execution.step(&mut host).unwrap() {}
    let encoded = serde_json::to_value(execution).unwrap();
    let mut variants = Vec::new();
    for offset in [0, 4, 8] {
        let mut changed = encoded.clone();
        let value = changed["state"]["owner"]["data"][offset].as_u64().unwrap();
        changed["state"]["owner"]["data"][offset] = serde_json::json!(value ^ 255);
        variants.push(changed);
    }
    let mut changed = encoded.clone();
    changed["state"]["backing"]["data"][0] = serde_json::json!(0);
    variants.push(changed);
    let mut changed = encoded;
    changed["phase"] = serde_json::json!("Allocate");
    variants.push(changed);
    for changed in variants {
        let mut restored: Edit = serde_json::from_value(changed).unwrap();
        let calls = host.calls.clone();
        assert!(restored.step(&mut host).is_err());
        assert!(restored.into_state().is_err());
        assert_eq!(host.calls, calls);
    }
}

#[test]
fn restored_default_constructor_cannot_enter_copy_phases_even_with_zero_allocator_bytes() {
    let mut host = Services {
        reply: Some(Allocated::Bytes {
            token: 0x900000,
            data: vec![0; 8],
        }),
        ..Services::default()
    };
    let mut execution = construction(None);
    while !execution.step(&mut host).unwrap() {}
    let encoded = serde_json::to_value(execution).unwrap();
    for phase in ["Copy", "End"] {
        let mut changed = encoded.clone();
        changed["phase"] = serde_json::json!(phase);
        let mut restored: Construction = serde_json::from_value(changed).unwrap();
        assert!(restored.step(&mut host).is_err());
        assert!(restored.into_state().is_err());
    }
}

#[test]
fn restored_short_assignment_cannot_enter_append_phase_with_an_unchanged_prefix() {
    let mut host = Services::default();
    let state = finish(construction(Some(b"abc".to_vec())), &mut host);
    let execution = Edit::new(state, Operation::Assign(b"a".to_vec())).unwrap();
    let mut changed = serde_json::to_value(execution).unwrap();
    changed["phase"] = serde_json::json!("Append");
    let mut restored: Edit = serde_json::from_value(changed).unwrap();
    assert!(restored.step(&mut host).is_err());
    assert!(restored.into_state().is_err());
}
