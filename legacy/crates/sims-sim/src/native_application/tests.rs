use super::*;

fn incoming() -> Incoming {
    let mut registry = Registry::<u32> {
        generation: u32::MAX - 2,
        ..Default::default()
    };
    let mut retained = vec![
        Owner {
            token: 0x21000000,
            data: b"Sims\0retained source".to_vec(),
        },
        Owner {
            token: 0x22000000,
            data: [
                vec![0x11, 0x22, 0x7a, 0x44, 0x55, 0x66, 0x59, 0x88],
                vec![0xb3; 88],
            ]
            .concat(),
        },
        Owner {
            token: 0x25000000,
            data: b"Global\\Sims-test\0retained".to_vec(),
        },
        Owner {
            token: 0x26000000,
            data: b"Sims-local\0retained".to_vec(),
        },
    ];
    // Authored post-prologue caller image: [ebp+8] initially carries the
    // argument pointer; ebp is derived from the [ebp-2c] temporary at+16.
    retained[1].data[68..72].copy_from_slice(&0x21000000_u32.to_le_bytes());
    for (token, offset) in [(0x23000000, 12_usize), (0x24000000, 8)] {
        let identity = (token + offset as u64) as u32;
        registry.construct(identity).unwrap();
        registry.bind(identity, 0x667123).unwrap();
        let mut owner = Owner {
            token,
            data: vec![0x6e; 64],
        };
        let n = &registry.nodes[&identity];
        let header: Vec<_> = [n.handler, n.next.unwrap_or(0), n.auxiliary, n.serial]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        owner.data[offset..offset + 16].copy_from_slice(&header);
        retained.push(owner);
    }
    Incoming {
        application: Owner {
            token: 0x10000000,
            data: (0..324).map(|i| ((i * 13 + 17) % 251) as u8).collect(),
        },
        retained,
        registry_globals: Owner {
            token: 0x6de858,
            data: [registry.head.unwrap(), registry.generation]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect(),
        },
        registry,
        application_singleton: Owner {
            token: 0x6f2a58,
            data: 0xaabbccdd_u32.to_le_bytes().to_vec(),
        },
        mutex_name_globals: Owner {
            token: 0x6c9f70,
            data: [0x25000000_u32, 0x26000000]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect(),
        },
        mutex_singleton: Owner {
            token: 0x6f2a5c,
            data: 0x11223344_u32.to_le_bytes().to_vec(),
        },
        mutex_temporary: 0x22000010,
        argument: 0x21000000,
        tree_stack: [0x22000002, 0x22000006],
    }
}

fn assert_retained_write_history(original: &Incoming, c: &Construction) {
    for (before, actual) in original.retained.iter().zip(c.retained_callers()) {
        let mut expected = before.clone();
        for write in c.writes().iter().filter(|write| write.owner == before.token) {
            assert_eq!(&expected.data[write.offset..write.offset + write.before.len()], write.before);
            expected.data[write.offset..write.offset + write.after.len()].copy_from_slice(&write.after);
        }
        assert_eq!(&expected, actual);
    }
}

#[derive(Default)]
struct TestHost {
    next: u64,
    live: BTreeMap<u64, Vec<u8>>,
    section_calls: usize,
    tree_calls: usize,
    string_calls: usize,
    block_section: bool,
    block_string: bool,
    bad_section: Option<InitializedSection>,
    bad_tree: Option<Allocated>,
    bad_string: Option<Allocated>,
    com_calls: usize,
    block_com: bool,
    failed_com: Option<u32>,
    bad_com: Option<com::Return>,
    version: Option<u32>,
    version_calls: usize,
    bad_version: Option<platform::VersionReturn>,
    wrapper_calls: usize,
    bad_wrapper: Option<Allocated>,
    block_wrapper: bool,
}
impl TestHost {
    fn allocation(&mut self, bytes: u32) -> Allocated {
        if self.next == 0 {
            self.next = 0x40000000;
        }
        let token = self.next;
        self.next += u64::from((bytes + 15) & !15);
        let data: Vec<_> = (0..bytes).map(|i| (0x91_u32 + i % 37) as u8).collect();
        self.live.insert(token, data.clone());
        Allocated::Bytes { token, data }
    }
}
impl strings::Host for TestHost {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        if self.block_string {
            return Err(Stop::Unavailable("test string pending".into()));
        }
        self.string_calls += 1;
        Ok(self
            .bad_string
            .take()
            .unwrap_or_else(|| self.allocation(request.bytes())))
    }
    fn free_string(&mut self, _: u64) -> Result<(), Stop> {
        panic!("prefix must retain its live backings")
    }
}
impl Host for TestHost {
    fn query_version(
        &mut self,
        request: &platform::VersionRequest,
    ) -> Result<platform::VersionReturn, Stop> {
        let raw = self
            .version
            .ok_or_else(|| Stop::Unavailable("test version pending".into()))?;
        self.version_calls += 1;
        match self.bad_version.take() {
            Some(reply) => Ok(reply),
            None => platform::VersionPolicy::RawDword(raw).reply(request),
        }
    }
    fn allocate_mutex_wrapper(
        &mut self,
        request: &platform::WrapperRequest,
    ) -> Result<Allocated, Stop> {
        if self.block_wrapper {
            return Err(Stop::Unavailable("test wrapper pending".into()));
        }
        request.validate()?;
        self.wrapper_calls += 1;
        Ok(self
            .bad_wrapper
            .take()
            .unwrap_or_else(|| self.allocation(request.bytes)))
    }
    fn initialize_com(&mut self, request: &com::Request) -> Result<com::Return, Stop> {
        if self.block_com {
            return Err(Stop::Unavailable("test COM call pending".into()));
        }
        self.com_calls += 1;
        if let Some(reply) = self.bad_com.take() {
            return Ok(reply);
        }
        match self.failed_com {
            Some(hresult) => com::Return::failed(request, hresult),
            None => com::initialize_rust_port(request),
        }
    }
    fn initialize_section(&mut self, request: &SectionRequest) -> Result<InitializedSection, Stop> {
        if self.block_section {
            return Err(Stop::Unavailable("test section pending".into()));
        }
        self.section_calls += 1;
        assert_eq!(request.region.data, vec![0; 24]);
        // Explicit synthetic nullable-debug outcome, not a native heap result.
        Ok(self
            .bad_section
            .take()
            .unwrap_or_else(|| InitializedSection::from_debug_info(request, None, 7).unwrap()))
    }
    fn allocate_tree(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        assert_eq!(bytes, 24);
        self.tree_calls += 1;
        Ok(self
            .bad_tree
            .take()
            .unwrap_or_else(|| self.allocation(bytes)))
    }
}

fn finish(construction: &mut Construction, host: &mut TestHost) {
    for _ in 0..4096 {
        if construction.step(host).unwrap() == Some(Cut::ImportEntry) {
            return;
        }
    }
    panic!("application prefix did not reach its explicit pending import");
}

#[test]
fn original_prefix_preserves_full_owners_registry_order_and_distinct_strings() {
    let input = incoming();
    let original = input.clone();
    let mut c = Construction::new(input).unwrap();
    let mut host = TestHost::default();
    finish(&mut c, &mut host);
    c.validate().unwrap();
    assert_eq!(c.incoming, original);
    assert_eq!(
        (host.section_calls, host.tree_calls, host.string_calls),
        (1, 1, 7)
    );
    assert_eq!(
        c.progress.strings.keys().copied().collect::<Vec<_>>(),
        [92, 164, 180, 192, 204, 216, 228]
    );
    assert_eq!(c.progress.strings[&92].bytes().unwrap(), b"Sims");
    assert_eq!(c.progress.strings[&92].backing.data.len(), 5);
    for offset in [164, 180, 192, 204, 216, 228] {
        let backing = &c.progress.strings[&offset].backing;
        assert_eq!(backing.data.len(), 8);
        assert_eq!(backing.data[0], 0);
        assert_eq!(&backing.data[1..], &host.live[&backing.token][1..]);
    }
    let tree = c.progress.tree.as_ref().unwrap();
    assert_eq!(tree.data.len(), 24);
    assert_eq!(&tree.data[1..4], &host.live[&tree.token][1..4]);
    assert_eq!(&tree.data[16..], &host.live[&tree.token][16..]);
    assert_eq!(word(&tree.data[8..]), tree.token as u32);
    assert_eq!(word(&tree.data[12..]), tree.token as u32);
    assert_eq!(c.owner().data[88], 0x7a);
    assert_eq!(&c.owner().data[89..92], &original.application.data[89..92]);
    assert_eq!(
        &c.owner().data[24..48],
        c.progress.section.as_ref().unwrap().region.data
    );
    let identity = (c.owner().token + 104) as u32;
    assert_eq!(c.registry().head, Some(identity));
    assert_eq!(c.registry().generation, 0);
    assert_eq!(c.registry().nodes[&identity].handler, 0x669804);
    assert_eq!(c.registry().nodes[&identity].next, original.registry.head);
    assert_eq!(c.registry().nodes[&identity].serial, u32::MAX);
    assert_eq!(word(&c.registry_globals().data[4..]), 0);
    let mut expected = original.application.data.clone();
    for write in c
        .writes()
        .iter()
        .filter(|write| write.owner == c.owner().token)
    {
        assert_eq!(
            &expected[write.offset..write.offset + write.before.len()],
            write.before
        );
        expected[write.offset..write.offset + write.after.len()].copy_from_slice(&write.after);
    }
    assert_eq!(expected, c.owner().data);
    for offset in [
        65, 66, 67, 89, 90, 91, 176, 248, 254, 255, 256, 280, 305, 314, 315, 316, 317, 318, 319,
        323,
    ] {
        assert_eq!(
            c.owner().data[offset],
            original.application.data[offset],
            "untouched byte{offset}"
        );
    }
    // Parent's private instruction audit binds this synthetic execution's exact
    // stores/call returns to the original offline disassembly, not a fixture.
    let writes:Vec<_>=c.writes().iter().map(|w|serde_json::json!({"owner":w.owner,"offset":w.offset,"pc":w.instruction,"child_return":w.child_return,"after":w.after})).collect();
    println!(
        "APPLICATION_PREFIX_WRITES {}",
        serde_json::to_string(&writes).unwrap()
    );
}

#[test]
fn unavailable_section_keeps_the_complete_incoming_and_same_pending_call() {
    let mut c = Construction::new(incoming()).unwrap();
    let mut host = TestHost {
        block_section: true,
        ..Default::default()
    };
    while !matches!(c.pending_call(), Some(Call::InitializeSection(_))) {
        c.step(&mut host).unwrap();
    }
    let before = c.progress.clone();
    let request = c.pending_call();
    for _ in 0..2 {
        assert!(matches!(c.step(&mut host), Err(Stop::Unavailable(_))));
        assert_eq!(c.progress, before);
        c.validate().unwrap();
    }
    assert_eq!(host.section_calls, 0);
    assert_eq!(c.pending_call(), request);
    c = match c.into_prefix() {
        Err(c) => c,
        Ok(_) => panic!("incomplete constructor extracted"),
    };
    host.block_section = false;
    finish(&mut c, &mut host);
    assert_eq!(host.section_calls, 1);
}

#[test]
fn malformed_successful_section_reply_is_retained_without_a_second_effect() {
    for change in 0..3 {
        let request = SectionRequest {
            instruction: 0x5d91bc,
            import_slot: 0x666114,
            region: Owner {
                token: 0x10000018,
                data: vec![0; 24],
            },
        };
        let mut reply = InitializedSection::from_debug_info(&request, None, 7).unwrap();
        match change {
            0 => reply.capability = 0,
            1 => reply.region.token += 1,
            _ => {
                reply.region.data.pop();
            }
        }
        let mut host = TestHost {
            bad_section: Some(reply.clone()),
            ..Default::default()
        };
        let mut c = Construction::new(incoming()).unwrap();
        for _ in 0..64 {
            if c.step(&mut host).is_err() {
                break;
            }
        }
        assert_eq!(c.progress.pending_section, Some(reply));
        assert_eq!(host.section_calls, 1);
        assert!(c.step(&mut host).is_err());
        assert_eq!(host.section_calls, 1);
    }
}

#[test]
fn invalid_tree_replies_keep_the_allocation_result_and_never_allocate_again() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x10000100,
            data: vec![0; 24],
        },
        Allocated::Bytes {
            token: u64::from(u32::MAX) - 8,
            data: vec![0; 24],
        },
        Allocated::Bytes {
            token: 0x40000000,
            data: vec![0; 23],
        },
    ] {
        let mut host = TestHost {
            bad_tree: Some(reply.clone()),
            ..Default::default()
        };
        let mut c = Construction::new(incoming()).unwrap();
        let mut rejected = false;
        for _ in 0..128 {
            if c.step(&mut host).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
        assert_eq!(host.tree_calls, 1);
        assert_eq!(c.progress.pending_tree, Some(reply));
        assert!(c.step(&mut host).is_err());
        assert_eq!(host.tree_calls, 1);
    }
}

#[test]
fn string_pending_alias_anywhere_in_the_full324_owner_is_rejected() {
    for token in [0x100000f0, 0x1000005c, 0x22000000] {
        let mut input = incoming();
        input.retained[0].data = b"12345678901\0retained".to_vec();
        let reply = Allocated::Bytes {
            token,
            data: vec![0; 12],
        };
        let mut host = TestHost {
            bad_string: Some(reply),
            ..Default::default()
        };
        let mut c = Construction::new(input).unwrap();
        let mut rejected = false;
        for _ in 0..256 {
            if c.step(&mut host).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
        assert_eq!(host.string_calls, 1);
        assert!(c.progress.child.is_some());
        assert!(c.step(&mut host).is_err());
        assert_eq!(host.string_calls, 1);
    }
}

#[test]
fn null_string_reply_is_retained_at_acceptance_and_pending_host_error_can_resume() {
    let mut c = Construction::new(incoming()).unwrap();
    let mut host = TestHost {
        bad_string: Some(Allocated::Null),
        ..Default::default()
    };
    for _ in 0..256 {
        if c.step(&mut host).is_err() {
            break;
        }
    }
    assert_eq!(host.string_calls, 1);
    let child = c.progress.child.clone();
    assert!(c.step(&mut host).is_err());
    assert_eq!(c.progress.child, child);
    assert_eq!(host.string_calls, 1);
    let mut c = Construction::new(incoming()).unwrap();
    let mut host = TestHost {
        block_string: true,
        ..Default::default()
    };
    for _ in 0..256 {
        if c.step(&mut host).is_err() {
            break;
        }
    }
    let before = c.progress.clone();
    assert!(c.step(&mut host).is_err());
    assert_eq!(c.progress, before);
    assert_eq!(host.string_calls, 0);
    host.block_string = false;
    finish(&mut c, &mut host);
    assert_eq!(host.string_calls, 7);
}

#[test]
fn exact_outer_progress_rejects_a_zero_byte_child_phase_jump() {
    let mut input = incoming();
    input.application.data[92..104].fill(0);
    let mut c = Construction::new(input).unwrap();
    let mut host = TestHost::default();
    while c.progress.child.is_none() {
        c.step(&mut host).unwrap();
    }
    let mut encoded = serde_json::to_value(c.progress.child.as_ref().unwrap()).unwrap();
    encoded["phase"] = serde_json::json!({"Clear":1});
    let child: strings::Construction = serde_json::from_value(encoded).unwrap();
    child.allocation_spans().unwrap(); // Local byte/phase consistency alone passes.
    c.progress.child = Some(child);
    assert!(c.validate().is_err());
}

#[test]
fn source_and_reply_history_changes_fail_validation() {
    for change in 0..3 {
        let mut c = Construction::new(incoming()).unwrap();
        finish(&mut c, &mut TestHost::default());
        match change {
            0 => c.progress.owner.data[323] ^= 1,
            1 => c.steps -= 1,
            _ => {
                c.effects.remove(0);
            }
        }
        assert!(c.validate().is_err());
    }
}

#[test]
fn unavailable_com_preserves_import_cut_global_and_extractable_prefix() {
    let mut c = Construction::new(incoming()).unwrap();
    let mut host = TestHost::default();
    host.block_com = true;
    finish(&mut c, &mut host);
    assert_eq!(
        c.pending_call(),
        Some(Call::CoInitialize(c.com_request.clone()))
    );
    let before = c.progress.clone();
    let effects = c.effects.clone();
    for _ in 0..2 {
        assert!(matches!(c.step(&mut host), Err(Stop::Unavailable(_))));
        assert_eq!(c.progress, before);
        assert_eq!(c.effects, effects);
        c.validate().unwrap();
    }
    let prefix = match c.into_prefix() {
        Ok(p) => p,
        Err(_) => panic!("pending prefix lost"),
    };
    assert_eq!(prefix.construction().cut(), Some(Cut::ImportEntry));
    prefix.into_construction().validate().unwrap();
}

fn finish_to_version(c: &mut Construction, host: &mut TestHost) {
    for _ in 0..4096 {
        if matches!(c.pending_call(), Some(Call::GetVersion(_))) {
            return;
        }
        c.step(host).unwrap();
    }
    panic!("application did not reach its pending version query");
}

fn finish_to_platform(c: &mut Construction, host: &mut TestHost) {
    for _ in 0..4096 {
        if matches!(
            c.pending_call(),
            Some(Call::CreateMutex(_) | Call::ReadControlWord { .. })
        ) {
            return;
        }
        c.step(host).unwrap();
    }
    panic!("application did not reach its platform boundary");
}

#[test]
fn version_branch_copies_the_owned_name_and_preserves_the_full_caller_until_mutex_entry() {
    for (raw, expected) in [
        (5, b"Global\\Sims-test".as_slice()),
        (0x80000005, b"Sims-local".as_slice()),
    ] {
        let input = incoming();
        let original = input.clone();
        let mut c = Construction::new(input).unwrap();
        let mut host = TestHost {
            version: Some(raw),
            ..Default::default()
        };
        finish_to_platform(&mut c, &mut host);
        let Some(Call::CreateMutex(request)) = c.pending_call() else {
            panic!("missing CreateMutex entry")
        };
        request.validate().unwrap();
        assert_eq!(request.name_bytes().unwrap(), expected);
        assert_eq!(request.name.owner.token, original.mutex_temporary);
        assert_eq!(&request.name.backing.data[..], &[expected, b"\0"].concat());
        assert!(
            !original
                .retained
                .iter()
                .any(|o| o.token == request.name.backing.token)
        );
        let wrapper = c.mutex_wrapper().unwrap();
        assert_eq!(wrapper.data.len(), 12);
        assert_eq!(word(&wrapper.data), 0x673b18);
        assert_eq!(word(&wrapper.data[8..]), 0);
        assert_eq!(&wrapper.data[4..8], &host.live[&wrapper.token][4..8]);
        assert_eq!(c.mutex_singleton(), &original.mutex_singleton);
        assert_retained_write_history(&original, &c);
        let container = c.progress.retained.iter().find(|owner| {
            original.mutex_temporary >= owner.token
                && original.mutex_temporary + 12 <= owner.token + owner.data.len() as u64
        }).unwrap();
        let at = (original.mutex_temporary - container.token) as usize;
        assert_eq!(&container.data[at..at + 12], request.name.owner.data);
        assert_eq!(
            (host.version_calls, host.wrapper_calls, host.string_calls),
            (1, 1, 8)
        );
        let before = c.progress.clone();
        for _ in 0..2 {
            assert!(c.step(&mut host).is_err());
            assert_eq!(c.progress, before);
            c.validate().unwrap();
        }
        assert_eq!(
            (host.version_calls, host.wrapper_calls, host.string_calls),
            (1, 1, 8)
        );
        assert!(c.into_prefix().is_ok());
    }
}

#[test]
fn null_wrapper_publishes_null_and_skips_temporary_and_mutex_creation() {
    for raw in [5, 0x80000005] {
        let input = incoming();
        let original = input.clone();
        let mut c = Construction::new(input).unwrap();
        let mut host = TestHost {
            version: Some(raw),
            bad_wrapper: Some(Allocated::Null),
            ..Default::default()
        };
        finish_to_platform(&mut c, &mut host);
        assert!(matches!(
            c.pending_call(),
            Some(Call::ReadControlWord {
                instruction: 0x4eedc7
            })
        ));
        assert_eq!(c.cut(), Some(Cut::NullMutexPublished));
        assert!(c.mutex_wrapper().is_none());
        assert!(c.progress.temporary.is_none());
        assert_eq!(c.mutex_singleton().data, 0_u32.to_le_bytes());
        assert_eq!(c.owner().data[248], 0);
        assert_retained_write_history(&original, &c);
        assert_eq!(
            (host.version_calls, host.wrapper_calls, host.string_calls),
            (1, 1, 7)
        );
        let writes: Vec<_> = c.writes().iter().filter(|w| w.owner == 0x6f2a5c).collect();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            writes[0].instruction,
            if raw == 5 { 0x4eed54 } else { 0x4eeda2 }
        );
        assert_eq!(writes[0].before, original.mutex_singleton.data);
        c.validate().unwrap();
    }
}

#[test]
fn unavailable_wrapper_preserves_version_cut_and_foreign_version_return_is_retained() {
    let mut c = Construction::new(incoming()).unwrap();
    let mut host = TestHost {
        version: Some(5),
        block_wrapper: true,
        ..Default::default()
    };
    while !matches!(c.pending_call(), Some(Call::AllocateMutexWrapper(_))) {
        c.step(&mut host).unwrap();
    }
    let before = c.progress.clone();
    let request = c.pending_call();
    for _ in 0..2 {
        assert!(c.step(&mut host).is_err());
        assert_eq!(c.progress, before);
        assert_eq!(c.pending_call(), request);
        c.validate().unwrap();
    }
    assert_eq!(c.cut(), Some(Cut::VersionReturned));
    host.block_wrapper = false;
    finish_to_platform(&mut c, &mut host);
    assert_eq!((host.version_calls, host.wrapper_calls), (1, 1));
    let other = Construction::new(incoming()).unwrap();
    let mut c = Construction::new(incoming()).unwrap();
    let mut host = TestHost {
        version: Some(5),
        bad_version: Some(
            platform::VersionPolicy::RawDword(5)
                .reply(&other.version_request)
                .unwrap(),
        ),
        ..Default::default()
    };
    finish_to_version(&mut c, &mut host);
    assert!(c.step(&mut host).is_err());
    assert!(c.progress.pending_version.is_some());
    assert!(c.step(&mut host).is_err());
    assert_eq!(host.version_calls, 1);
    assert_eq!(host.wrapper_calls, 0);
}

#[test]
fn pending_platform_allocations_and_changed_caller_padding_fail_whole_owner_replay() {
    for token in [0x6c9f70, 0x6f2a5c, 0x22000030, 0x26000000] {
        let mut c = Construction::new(incoming()).unwrap();
        let mut host = TestHost {
            version: Some(5),
            bad_wrapper: Some(Allocated::Bytes {
                token,
                data: vec![0; 12],
            }),
            ..Default::default()
        };
        while !matches!(c.pending_call(), Some(Call::AllocateMutexWrapper(_))) {
            c.step(&mut host).unwrap();
        }
        assert!(c.step(&mut host).is_err());
        assert!(c.step(&mut host).is_err());
        assert_eq!(host.wrapper_calls, 1);
        assert!(c.progress.pending_wrapper.is_some());
    }
    let mut c = Construction::new(incoming()).unwrap();
    finish_to_platform(
        &mut c,
        &mut TestHost {
            version: Some(5),
            ..Default::default()
        },
    );
    c.progress.retained[1].data[63] ^= 1;
    assert!(c.validate().is_err());
    let mut input = incoming();
    input.mutex_temporary = 0x25000000;
    assert!(Construction::new(input).is_err());
}

#[test]
fn mutex_temporary_cannot_reuse_a_retained_commanders_containing_owner() {
    for offset in [12, 0, 40] {
        let mut input = incoming();
        input.mutex_temporary = 0x23000000 + offset;
        let argument = input.argument as u32;
        let commander = input.retained.iter_mut().find(|owner| owner.token == 0x23000000).unwrap();
        commander.data.resize(128, 0x6e);
        let argument_slot = offset as usize + 52;
        commander.data[argument_slot..argument_slot + 4].copy_from_slice(&argument.to_le_bytes());
        let before = input.clone();
        let error = Construction::new(input)
            .expect_err("mutable temporary must have a separate containing stack owner");
        assert_eq!(error.incoming, before);
        assert!(matches!(error.stop, Stop::Unsupported(ref message) if message.contains("retained Commander's containing owner")));
    }
}

#[test]
fn every_acknowledged_hresult_publishes_once_then_preserves_the_version_boundary() {
    for expected in [
        0,
        1,
        0x80070057,
        0x8007000e,
        0x80004005,
        0x80010106,
        u32::MAX,
    ] {
        let input = incoming();
        let preimage = input.application_singleton.clone();
        let mut c = Construction::new(input).unwrap();
        let prior = if expected == 1 {
            Some(com::initialize_rust_port(&c.com_request).unwrap())
        } else {
            None
        };
        let mut host = TestHost {
            failed_com: (expected & 0x8000_0000 != 0).then_some(expected),
            ..Default::default()
        };
        finish(&mut c, &mut host);
        assert_eq!(c.application_singleton(), &preimage);
        assert!(c.com_initialization().is_none());
        finish_to_version(&mut c, &mut host);
        let returned = c.com_initialization().unwrap();
        assert_eq!(returned.hresult(), expected);
        assert_eq!(
            returned.current_apartment().is_ok(),
            expected & 0x8000_0000 == 0
        );
        assert_eq!(
            c.application_singleton().data,
            (c.owner().token as u32).to_le_bytes()
        );
        let writes: Vec<_> = c
            .writes()
            .iter()
            .filter(|w| w.owner == preimage.token)
            .collect();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].instruction, 0x4eed08);
        assert_eq!(writes[0].before, preimage.data);
        let before = c.progress.clone();
        for _ in 0..2 {
            assert!(matches!(c.step(&mut host), Err(Stop::Unavailable(_))));
            assert_eq!(c.progress, before);
            c.validate().unwrap();
        }
        assert_eq!(host.com_calls, 1);
        assert_eq!(c.cut(), Some(Cut::Published));
        let prefix = c.into_prefix().unwrap();
        prefix.into_construction().validate().unwrap();
        drop(prior);
    }
}

#[test]
fn another_identical_address_invocation_cannot_supply_the_com_return() {
    let other = Construction::new(incoming()).unwrap();
    let returned = com::initialize_rust_port(&other.com_request).unwrap();
    let mut c = Construction::new(incoming()).unwrap();
    assert_ne!(c.com_request, other.com_request);
    let mut host = TestHost {
        bad_com: Some(returned.clone()),
        ..Default::default()
    };
    finish(&mut c, &mut host);
    assert!(c.step(&mut host).is_err());
    assert_eq!(host.com_calls, 1);
    assert!(c.progress.pending_com.is_some());
    assert_eq!(c.application_singleton(), &c.incoming.application_singleton);
    assert!(c.step(&mut host).is_err());
    assert_eq!(host.com_calls, 1);
    assert_eq!(
        returned.current_apartment().unwrap().initialization_count(),
        1
    );
}

#[test]
fn singleton_ownership_and_publication_history_remain_part_of_exact_replay() {
    let mut aliased = incoming();
    aliased.retained.push(aliased.application_singleton.clone());
    assert!(Construction::new(aliased).is_err());
    let mut c = Construction::new(incoming()).unwrap();
    let mut host = TestHost {
        bad_tree: Some(Allocated::Bytes {
            token: 0x6f2a58,
            data: vec![0; 24],
        }),
        ..Default::default()
    };
    assert!((0..4096).any(|_| c.step(&mut host).is_err()));
    assert!(c.validate().is_err());
    for change in 0..4 {
        let mut c = Construction::new(incoming()).unwrap();
        finish_to_version(&mut c, &mut TestHost::default());
        match change {
            0 => c.incoming.application_singleton.data[0] ^= 1,
            1 => c.progress.singleton.data[0] ^= 1,
            2 => c.com_request.return_pc ^= 1,
            _ => c.progress.writes.last_mut().unwrap().instruction ^= 1,
        }
        assert!(c.validate().is_err());
    }
}

#[test]
fn begin_error_returns_all_inputs_without_discarding_a_containing_owner() {
    let mut input = incoming();
    input.registry_globals.data[4] ^= 1;
    let original = input.clone();
    let error = Construction::new(input).unwrap_err();
    assert_eq!(error.incoming, original);
}

#[test]
fn section_debug_allocation_cannot_alias_the_containing_application() {
    let request = SectionRequest {
        instruction: 0x5d91bc,
        import_slot: 0x666114,
        region: Owner {
            token: 0x10000018,
            data: vec![0; 24],
        },
    };
    let debug = Owner {
        token: 0x100000f0,
        data: vec![0; 32],
    };
    let reply = InitializedSection::from_debug_info(&request, Some(&debug), 7).unwrap();
    let mut host = TestHost {
        bad_section: Some(reply.clone()),
        ..Default::default()
    };
    let mut c = Construction::new(incoming()).unwrap();
    let mut rejected = false;
    for _ in 0..128 {
        if c.step(&mut host).is_err() {
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    assert_eq!(c.progress.pending_section, Some(reply));
    assert_eq!((host.section_calls, host.tree_calls), (1, 0));
    assert!(c.step(&mut host).is_err());
    assert_eq!(host.section_calls, 1);
}

#[test]
fn accepted_section_debug_owner_guards_later_string_allocation_replies() {
    let request = SectionRequest {
        instruction: 0x5d91bc,
        import_slot: 0x666114,
        region: Owner {
            token: 0x10000018,
            data: vec![0; 24],
        },
    };
    let debug = Owner {
        token: 0x50000000,
        data: vec![0; 32],
    };
    let reply = InitializedSection::from_debug_info(&request, Some(&debug), 7).unwrap();
    let mut host = TestHost {
        bad_section: Some(reply),
        bad_string: Some(Allocated::Bytes {
            token: 0x50000008,
            data: vec![0; 5],
        }),
        ..Default::default()
    };
    let mut c = Construction::new(incoming()).unwrap();
    let mut rejected = false;
    for _ in 0..256 {
        if c.step(&mut host).is_err() {
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    assert!(c.section().unwrap().debug_info.is_some());
    assert_eq!(host.string_calls, 1);
    assert!(c.step(&mut host).is_err());
    assert_eq!(host.string_calls, 1);
}

#[test]
fn caller_frame_source_stores_track_both_branches_and_null_allocation() {
    for nt in [false, true] {
        for null in [false, true] {
            let original = incoming();
            let frame = original.frame().unwrap();
            let mut c = Construction::new(original.clone()).unwrap();
            let mut host = TestHost {
                version: Some(if nt { 5 } else { 0x80000005 }),
                bad_wrapper: null.then_some(Allocated::Null),
                ..Default::default()
            };
            finish_to_platform(&mut c, &mut host);
            assert_retained_write_history(&original, &c);
            let slot = |relative| word(c.progress.bytes(frame.address(relative, 4).unwrap()).unwrap());
            assert_eq!(slot(-0x14), c.owner().token as u32);
            assert_eq!(slot(-0x10), if null { 0 } else if nt { 1 } else { 2 });
            assert_eq!(slot(-4), if null { 7 } else if nt { 9 } else { 12 });
            assert_eq!(slot(8), c.mutex_wrapper().map_or(0, |owner| owner.token as u32));
            assert_eq!(word(original.bytes(frame.address(8, 4).unwrap()).unwrap()), original.argument as u32);
            let mut expected_pcs = vec![
                0x4eebb1, 0x4eebb4, 0x4eebbf, 0x4eec05, 0x4eec1a,
                0x4eec29, 0x4eec38, 0x4eec47, 0x4eec56, 0x4eecc4,
                if nt { 0x4eed1f } else { 0x4eed6d },
                if nt { 0x4eed24 } else { 0x4eed72 },
            ];
            if null {
                expected_pcs.push(if nt { 0x4eed59 } else { 0x4eeda7 });
            } else {
                expected_pcs.extend(if nt {
                    [0x4eed33, 0x4eed38, 0x4eed43]
                } else {
                    [0x4eed81, 0x4eed86, 0x4eed91]
                });
            }
            let container = frame.containing(&original.retained).unwrap();
            let writes: Vec<_> = c.writes().iter().filter(|write| write.owner == container.token).collect();
            assert_eq!(writes.iter().map(|write| write.instruction).collect::<Vec<_>>(), expected_pcs);
            let allowed: [(u64, u64); 5] = [
                (original.mutex_temporary, 12),
                (frame.address(-0x14, 4).unwrap(), 4),
                (frame.address(-0x10, 4).unwrap(), 4),
                (frame.address(-4, 4).unwrap(), 4),
                (frame.address(8, 4).unwrap(), 4),
            ];
            let actual = frame.containing(c.retained_callers()).unwrap();
            for (i, (&before, &after)) in container.data.iter().zip(&actual.data).enumerate() {
                let address = container.token + i as u64;
                if !allowed.iter().any(|(start, bytes)| (*start..*start + *bytes).contains(&address)) {
                    assert_eq!(before, after, "caller padding changed at{address:x}");
                }
            }
            let before = c.progress.clone();
            for _ in 0..2 {
                assert!(c.step(&mut host).is_err());
                c.validate().unwrap();
                assert_eq!(c.progress, before);
            }
        }
    }
}

#[test]
fn caller_frame_needs_one_complete_owner_and_original_argument_slot() {
    let mut truncated = incoming();
    truncated.retained[1].data.truncate(71);
    assert!(Construction::new(truncated).is_err());
    let mut split = incoming();
    let suffix = split.retained[1].data.split_off(48);
    split.retained.push(Owner { token: 0x22000030, data: suffix });
    assert!(Construction::new(split).is_err());
    let mut wrong_argument = incoming();
    wrong_argument.retained[1].data[68..72].copy_from_slice(&0_u32.to_le_bytes());
    assert!(matches!(Construction::new(wrong_argument).unwrap_err().stop,
        Stop::Unsupported(ref message) if message.contains("argument slot")));
    assert!(Frame::new(u64::MAX).is_err());
    assert!(Frame::new((1_u64 << 32) - 55).is_err());
}

#[test]
fn caller_operands_read_live_stack_bytes_after_prior_source_stores() {
    let mut input = incoming();
    input.argument = 0x22000028;
    input.retained[1].data[40..45].copy_from_slice(b"kept\0");
    input.retained[1].data[68..72].copy_from_slice(&(input.argument as u32).to_le_bytes());
    input.tree_stack[0] = 0x22000028;
    assert_eq!(input.argument_bytes().unwrap(), b"kept");
    let mut c = Construction::new(input).unwrap();
    finish(&mut c, &mut TestHost::default());
    // 4eebb1 writes the application pointer before Base104 copies the
    // argument or its tree child reads this caller-supplied byte view.
    assert_eq!(c.progress.strings[&92].bytes().unwrap(), b"");
    assert_eq!(c.owner().data[88], 0);
    c.validate().unwrap();
}

#[test]
fn mutex_name_copy_reads_live_caller_storage_before_branch_flag_reuses_it() {
    let mut input = incoming();
    input.mutex_name_globals.data[..4].copy_from_slice(&0x2200002c_u32.to_le_bytes());
    assert!(!input.c_string(0x2200002c).unwrap().is_empty());
    let mut c = Construction::new(input).unwrap();
    finish_to_platform(&mut c, &mut TestHost { version: Some(5), ..Default::default() });
    let Some(Call::CreateMutex(request)) = c.pending_call() else { panic!("missing mutex entry") };
    // The initial flag store zeroes this CString before the copy. The later
    // branch flag then changes the caller bytes, not the already-owned copy.
    assert_eq!(request.name_bytes().unwrap(), b"");
    assert_eq!(word(c.progress.bytes(0x2200002c).unwrap()), 1);
    c.validate().unwrap();
}
