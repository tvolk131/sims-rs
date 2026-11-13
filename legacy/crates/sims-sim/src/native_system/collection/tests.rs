//! Authored source tests, UNRUN. FactoryHost and SyntheticHost are explicitly
//! synthetic; no native heap, runtime lock, character or gameplay claim follows.
use super::*;

pub(super) fn event_fixture() -> Construction {
    let mut cursor = Construction::new(incoming()).unwrap();
    finish(&mut cursor, &mut SyntheticHost::default());
    cursor
}

fn put(owner: &mut Owner, at: usize, value: u32) {
    owner.data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
struct FactoryHost {
    trees: usize,
}
impl factory::Host for FactoryHost {
    fn allocate_provider(&mut self, request: &factory::Request) -> Result<Allocated, Stop> {
        request.validate()?;
        Ok(Allocated::Bytes {
            token: 0x1000_0000,
            data: (0..1240).map(|at| (at * 19 + 0x73) as u8).collect(),
        })
    }
    fn allocate_tree(&mut self, request: &factory::TreeRequest) -> Result<Allocated, Stop> {
        request.validate()?;
        let token = 0x3000_0000 + self.trees as u64 * 0x100;
        self.trees += 1;
        Ok(Allocated::Bytes {
            token,
            data: vec![0xb6; 24],
        })
    }
    fn prepare_base72(&mut self, cut: &factory::Checkpoint) -> Result<(), Stop> {
        cut.validate()
    }
    fn initialize_base72(
        &mut self,
        call: &factory::Base72Call,
    ) -> Result<InitializedSection, Stop> {
        call.validate()?;
        InitializedSection::from_debug_info(
            &call.request().section_request(),
            Some(&Owner {
                token: 0x4000_0000,
                data: vec![0x91; 32],
            }),
            42,
        )
    }
    fn acknowledge_base72(
        &mut self,
        call: &factory::Base72Call,
        prefix: &super::super::base72::Prefix,
    ) -> Result<(), Stop> {
        call.validate()?;
        prefix.validate()
    }
    fn synchronize_checkpoint(&mut self, cut: &factory::Checkpoint) -> Result<(), Stop> {
        cut.validate()
    }
}
fn factory_cut(frontier: bool) -> factory::Checkpoint {
    let caller = Owner {
        token: 0x2000_0000,
        data: (0..1024).map(|at| (at * 37 + 0x91) as u8).collect(),
    };
    let b = caller.token;
    let mut cursor = factory::Construction::new(factory::Incoming {
        origin: factory::HeapOrigin::new(),
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
    .unwrap();
    let mut host = FactoryHost { trees: 0 };
    for _ in 0..512 {
        if cursor.step(&mut host).unwrap() == Some(factory::Cut::FirstDerivedChildReturned) {
            if frontier {
                assert_eq!(cursor.step(&mut host).unwrap(), None); // actual5d5875
                assert_eq!(
                    cursor.pending_call(),
                    Some(factory::Call::Collection {
                        instruction: 0x5d5879
                    })
                );
            }
            return cursor.checkpoint().unwrap();
        }
    }
    panic!("synthetic factory did not reach its bounded source frontier");
}
fn incoming() -> Incoming {
    let factory = factory_cut(true);
    let b = factory.caller().token;
    Incoming {
        factory,
        frames: Frames {
            collection: b + 384,
            list: b + 448,
            section_base: b + 512,
            resource_argument: b + 560,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Sync(usize),
    List,
    Section(SectionSite),
}
#[derive(Default)]
struct SyntheticHost {
    events: Vec<Event>,
    bound: Option<Request>,
    high_water: usize,
    list_attempts: usize,
    section_attempts: usize,
    list_effects: usize,
    section_effects: usize,
    lists: Vec<ListRequest>,
    calls: Vec<SectionCall>,
    replies: Vec<InitializedSection>,
    null_debug: [bool; 3],
    list_override: Option<Allocated>,
    section_fault: Option<fn(&SectionCall, &mut InitializedSection)>,
    fault_site: Option<SectionSite>,
    swapped: Option<InitializedSection>,
    fail_sync: bool,
    fail_list: bool,
    fail_section: Option<SectionSite>,
    list_after_effect: bool,
    section_after_effect: Option<SectionSite>,
    pending_list: Option<(ListRequest, Allocated)>,
    pending_section: Option<(SectionCall, InitializedSection)>,
}
impl Host for SyntheticHost {
    fn synchronize_checkpoint(&mut self, cut: &Checkpoint) -> Result<(), Stop> {
        cut.validate()?;
        self.events.push(Event::Sync(cut.source_ordinal()));
        if self.fail_sync {
            self.fail_sync = false;
            return Err(unavailable("synthetic synchronization quota"));
        }
        if let Some(bound) = &self.bound {
            if !bound.same_call(cut.request()) || cut.source_ordinal() < self.high_water {
                return Err(unsupported(
                    "synthetic host rejected foreign invocation/stale rewind",
                ));
            }
        } else {
            self.bound = Some(cut.request().clone());
        }
        self.high_water = cut.source_ordinal();
        Ok(())
    }
    fn allocate_list(&mut self, request: &ListRequest) -> Result<Allocated, Stop> {
        request.validate()?;
        assert_eq!(request.bytes(), 24);
        assert_eq!(request.instruction(), 0x6017c8);
        assert_eq!(
            self.events.last(),
            Some(&Event::Sync(request.checkpoint().source_ordinal()))
        );
        self.events.push(Event::List);
        self.list_attempts += 1;
        self.lists.push(request.clone());
        if self.fail_list {
            self.fail_list = false;
            return Err(unavailable("synthetic list quota before effect"));
        }
        if let Some((call, reply)) = self.pending_list.take() {
            assert!(call.same_call(request));
            return Ok(reply);
        }
        self.list_effects += 1;
        let reply = self
            .list_override
            .take()
            .unwrap_or_else(|| Allocated::Bytes {
                token: 0x5000_0000,
                data: (0..24).map(|at| (0xd3 + at * 7) as u8).collect(),
            });
        if self.list_after_effect {
            self.list_after_effect = false;
            self.pending_list = Some((request.clone(), reply));
            return Err(unavailable("synthetic host retains list after effect"));
        }
        Ok(reply)
    }
    fn initialize_section(&mut self, call: &SectionCall) -> Result<InitializedSection, Stop> {
        call.validate()?;
        assert_eq!(
            self.events.last(),
            Some(&Event::Sync(call.checkpoint().source_ordinal()))
        );
        self.events.push(Event::Section(call.site()));
        self.section_attempts += 1;
        self.calls.push(call.clone());
        if self.fail_section == Some(call.site()) {
            self.fail_section = None;
            return Err(unavailable("synthetic section quota before effect"));
        }
        if let Some((previous, reply)) = self.pending_section.take() {
            assert!(previous.same_call(call));
            return Ok(reply);
        }
        self.section_effects += 1;
        let index = call.site().index();
        let debug = (!self.null_debug[index]).then(|| Owner {
            token: 0x6000_0000 + index as u64 * 0x100,
            data: vec![0x7e; 32],
        });
        let mut reply = InitializedSection::from_debug_info(
            &call.section_request(),
            debug.as_ref(),
            100 + index as u64,
        )?;
        if self.fault_site == Some(call.site()) {
            if let Some(swapped) = self.swapped.take() {
                reply = swapped;
            }
            if let Some(fault) = self.section_fault {
                fault(call, &mut reply);
            }
        }
        self.replies.push(reply.clone());
        if self.section_after_effect == Some(call.site()) {
            self.section_after_effect = None;
            self.pending_section = Some((call.clone(), reply));
            return Err(unavailable("synthetic host retains section after effect"));
        }
        Ok(reply)
    }
}
fn reach(cursor: &mut Construction, host: &mut SyntheticHost, call: Call) {
    for _ in 0..96 {
        if cursor.pending_call() == Some(call) {
            return;
        }
        assert_eq!(cursor.step(host).unwrap(), None);
    }
    panic!("synthetic cursor did not reach requested bounded call");
}
fn finish(cursor: &mut Construction, host: &mut SyntheticHost) -> Checkpoint {
    for _ in 0..96 {
        if cursor.step(host).unwrap() == Some(Cut::BeforeResource) {
            return cursor.checkpoint().unwrap();
        }
    }
    panic!("synthetic cursor did not stop before resource import");
}

#[test]
fn collection_exact_order_widths_complete_images_and_untouched_suffix() {
    let input = incoming();
    let original_parent = input.factory.parent().clone();
    let original_caller = input.factory.caller().clone();
    let frames = input.frames;
    assert_ne!(word(&original_caller.data[560..564]), 0);
    assert_ne!(word(&original_parent.data[260..264]), 0);
    let mut cursor = Construction::new(input).unwrap();
    let mut host = SyntheticHost::default();
    let cut = finish(&mut cursor, &mut host);
    assert_eq!((host.list_effects, host.section_effects), (1, 3));
    assert_eq!(cut.source_ordinal(), 39);
    assert_eq!(
        cut.writes()
            .iter()
            .map(|w| w.instruction)
            .collect::<Vec<_>>(),
        vec![
            0x5d5d05, 0x5d5d0b, 0x5d91a7, 0x5d91aa, 0x5d91ad, 0x5d91b3, 0x5d91bc, 0x5d91c2,
            0x5d5d16, 0x6017bf, 0x6017c2, 0x6017cd, 0x6017d0, 0x6017d6, 0x6017d9, 0x5d91a7,
            0x5d91aa, 0x5d91ad, 0x5d91b3, 0x5d91bc, 0x5d91c2, 0x6017e5, 0x6017ea, 0x5d9310,
            0x5d9470, 0x5d91a7, 0x5d91aa, 0x5d91ad, 0x5d91b3, 0x5d91bc, 0x5d91c2, 0x5d947f,
            0x5d9485, 0x5d9492, 0x5d9321, 0x5d9329, 0x5d9333, 0x5d9336,
        ]
    );
    let mut expected = original_parent.clone();
    for (offset, value) in [
        (128, 0x673884),
        (132, 0x673ab8),
        (160, 1),
        (164, 0),
        (168, 0x676a00),
        (172, 0x5000_0000),
        (176, 0x673ab8),
        (204, 1),
        (208, 0),
        (212, 0x673adc),
        (216, 0x673af8),
        (220, 0x673ab8),
        (248, 1),
        (252, 0),
        (256, 1),
        (264, 0),
        (268, 0),
    ] {
        put(&mut expected, offset, value);
    }
    for (index, at) in [136, 180, 224].into_iter().enumerate() {
        let section = cut.sections()[index].as_ref().unwrap();
        assert_eq!(section, &host.replies[index]);
        expected.data[at..at + 24].copy_from_slice(&section.region.data);
    }
    assert_eq!(cut.parent(), &expected);
    let mut caller = original_caller.clone();
    for (address, value) in [
        (frames.collection - 20, (expected.token + 128) as u32),
        (frames.collection - 4, 0),
        (frames.list - 16, (expected.token + 168) as u32),
        (frames.list - 4, 1),
        (frames.section_base - 16, (expected.token + 216) as u32),
        (frames.section_base - 4, 0),
        (frames.resource_argument, 0),
    ] {
        let at = (address - caller.token) as usize;
        put(&mut caller, at, value);
    }
    assert_eq!(cut.caller_frame(), &caller);
    let mut list = cut.initial_list().unwrap().clone();
    let list_token = list.token as u32;
    put(&mut list, 0, list_token);
    put(&mut list, 4, list_token);
    assert_eq!(cut.list(), Some(&list));
    assert_eq!(&list.data[8..], &cut.initial_list().unwrap().data[8..]);
    assert_eq!(
        &cut.parent().data[260..264],
        &original_parent.data[260..264]
    );
    assert_eq!(&cut.parent().data[272..], &original_parent.data[272..]);
    assert_eq!(&cut.parent().data[..128], &original_parent.data[..128]);
    // This DWORD is SpinCount inside section180..204, not allocator padding.
    assert_ne!(&original_parent.data[200..204], &[0; 4]);
    assert_eq!(&cut.parent().data[200..204], &[0; 4]);
    assert_eq!(
        cursor.pending_call(),
        Some(Call::Resource {
            instruction: 0x5d9339,
            import_slot: 0x666238,
            arguments: [0, 0, 0, 0],
        })
    );
    // Independent reversal recovers the complete factory cut and allocator
    // preimage, including every untouched byte, rather than just modeled fields.
    let mut parent = cut.parent().clone();
    let mut caller = cut.caller_frame().clone();
    let mut list = cut.list().unwrap().clone();
    for write in cut.writes().iter().rev() {
        let owner = if write.owner == parent.token {
            &mut parent
        } else if write.owner == caller.token {
            &mut caller
        } else {
            assert_eq!(write.owner, list.token);
            &mut list
        };
        let bytes = &mut owner.data[write.offset..write.offset + write.after.len()];
        assert_eq!(&*bytes, write.after.as_slice());
        bytes.copy_from_slice(&write.before);
    }
    assert_eq!(parent, original_parent);
    assert_eq!(caller, original_caller);
    assert_eq!(&list, cut.initial_list().unwrap());
    for write in cut.writes() {
        let section = [0x5d91b3, 0x5d91bc].contains(&write.instruction);
        assert_eq!(write.child_return, section);
        assert_eq!(
            write.after.len(),
            if section {
                24
            } else if write.instruction == 0x6017ea {
                1
            } else {
                4
            }
        );
    }
    let before = host.events.clone();
    for _ in 0..3 {
        assert!(matches!(cursor.step(&mut host), Err(Stop::Unavailable(_))));
    }
    assert_eq!(host.events, before);
}

#[test]
fn all_three_calls_have_distinct_sealed_identities_and_stage_before_merge() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    for site in SITES {
        reach(&mut cursor, &mut host, Call::InitializeSection(site));
        let before = cursor.checkpoint().unwrap();
        cursor.step(&mut host).unwrap();
        let staged = cursor.checkpoint().unwrap();
        assert_eq!(staged.source_ordinal(), before.source_ordinal());
        assert_eq!(staged.parent(), before.parent());
        assert_eq!(staged.writes(), before.writes());
        assert!(before.sections()[site.index()].is_none());
        assert_eq!(
            staged.sections()[site.index()].as_ref(),
            host.replies.last()
        );
        assert_eq!(
            host.calls.last().unwrap().section_request().region.data,
            [0; 24]
        );
        assert_eq!(
            host.calls.last().unwrap().section_request().region.token,
            staged.parent().token + [136, 180, 224][site.index()]
        );
        cursor.step(&mut host).unwrap();
        let merged = cursor.checkpoint().unwrap();
        assert_eq!(merged.source_ordinal(), before.source_ordinal() + 1);
        assert_eq!(
            merged.writes().last().unwrap().after,
            host.replies.last().unwrap().region.data
        );
    }
    let cut = finish(&mut cursor, &mut host);
    for (at, call) in host.calls.iter().enumerate() {
        assert_eq!(call.site(), SITES[at]);
        assert!(call.same_call(&call.clone()));
        for other in host.calls.iter().skip(at + 1) {
            assert!(!call.same_call(other));
        }
    }
    assert!(host.lists[0].same_call(&host.lists[0].clone()));
    let mut second = Construction::new(Incoming {
        factory: cursor.request().factory().clone(),
        frames: cursor.request().frames(),
    })
    .unwrap();
    let mut other = SyntheticHost::default();
    let second_cut = finish(&mut second, &mut other);
    assert_eq!(cut.parent(), second_cut.parent());
    assert!(!cursor.request().same_call(second.request()));
    assert!(!host.lists[0].same_call(&other.lists[0]));
    for index in 0..3 {
        assert!(!host.calls[index].same_call(&other.calls[index]));
    }
}

#[test]
fn list_success_is_staged_without_cloning_or_publishing_allocator_bytes() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    reach(&mut cursor, &mut host, Call::AllocateList);
    let original = cursor.checkpoint().unwrap();
    cursor.step(&mut host).unwrap();
    let actual_storage = cursor.state.initial_list.as_ref().unwrap().data.as_ptr();
    let staged = cursor.checkpoint().unwrap();
    assert!(staged.list().is_none());
    assert!(staged.initial_list().is_some());
    assert_eq!(staged.parent(), original.parent());
    assert_eq!(staged.writes(), original.writes());
    assert_eq!(cursor.pending_call(), Some(Call::AllocateList));
    cursor.step(&mut host).unwrap();
    assert_eq!(
        cursor.state.initial_list.as_ref().unwrap().data.as_ptr(),
        actual_storage
    );
    assert_eq!(cursor.checkpoint().unwrap().list(), staged.initial_list());
    assert_eq!((host.list_attempts, host.list_effects), (1, 1));
}

#[test]
fn malformed_null_short_wrapping_and_aliased_list_returns_remain_owned() {
    let factory = factory_cut(true);
    let b = factory.caller().token;
    let frames = Frames {
        collection: b + 384,
        list: b + 448,
        section_base: b + 512,
        resource_argument: b + 560,
    };
    let tokens = [
        0,
        (1_u64 << 32) - 23,
        u64::MAX - 1,
        factory.parent().token + 1100,
        factory.caller().token + 800,
        factory.tree(factory::TreeSite::Derived).unwrap().token,
        factory
            .base72()
            .unwrap()
            .section()
            .debug_info
            .as_ref()
            .unwrap()
            .token,
        0x6000_0000,
    ]; // first new section's complete debug allocation
    let mut returns = vec![
        Allocated::Null,
        Allocated::Bytes {
            token: 0x5000_0000,
            data: vec![0x85; 23],
        },
        Allocated::Bytes {
            token: 0x5000_0000,
            data: vec![0x85; 25],
        },
    ];
    returns.extend(tokens.into_iter().map(|token| Allocated::Bytes {
        token,
        data: vec![0x85; 24],
    }));
    for reply in returns {
        let mut cursor = Construction::new(Incoming {
            factory: factory.clone(),
            frames,
        })
        .unwrap();
        let mut host = SyntheticHost::default();
        reach(&mut cursor, &mut host, Call::AllocateList);
        let parent = cursor.state.parent.clone();
        let writes = cursor.state.writes().to_vec();
        host.list_override = Some(reply.clone());
        assert!(cursor.step(&mut host).is_err());
        match &reply {
            Allocated::Null => assert!(cursor.state.initial_list.is_none()),
            Allocated::Bytes { token, data } => {
                let retained = cursor.state.initial_list.as_ref().unwrap();
                assert_eq!(retained.token, *token);
                assert_eq!(&retained.data, data);
            }
        }
        assert!(cursor.state.list_returned);
        assert!(cursor.checkpoint().is_err());
        for _ in 0..3 {
            assert!(cursor.step(&mut host).is_err());
        }
        assert_eq!(host.list_attempts, 1);
        assert_eq!(host.list_effects, 1);
        assert_eq!(cursor.state.parent, parent);
        assert_eq!(cursor.state.writes(), writes);
    }
}

#[test]
fn section_invalid_returns_cannot_be_replaced_on_any_of_the_three_sites() {
    let faults: [fn(&SectionCall, &mut InitializedSection); 12] = [
        |_, r| r.capability = 0,
        |_, r| r.capability = 42, // still-live first Base72 capability
        |_, r| r.region.token += 4,
        |_, r| {
            r.region.data.pop();
        },
        |_, r| r.region.data[4] = 0,
        |_, r| r.debug_info = None,
        |_, r| r.debug_info.as_mut().unwrap().data[4] ^= 1,
        |_, r| {
            r.debug_info.as_mut().unwrap().data.pop();
        },
        |_, r| r.debug_info.as_mut().unwrap().token = 0,
        |_, r| r.debug_info.as_mut().unwrap().token = (1_u64 << 32) - 31,
        |_, r| r.debug_info.as_mut().unwrap().token = u64::MAX - 4,
        |_, r| r.region.data.push(0),
    ];
    let input = incoming();
    for site in SITES {
        for fault in faults {
            let mut cursor = Construction::new(Incoming {
                factory: input.factory.clone(),
                frames: input.frames,
            })
            .unwrap();
            let mut host = SyntheticHost::default();
            reach(&mut cursor, &mut host, Call::InitializeSection(site));
            let before = cursor.checkpoint().unwrap();
            host.fault_site = Some(site);
            host.section_fault = Some(fault);
            assert!(cursor.step(&mut host).is_err());
            let actual = cursor.state.sections[site.index()].clone().unwrap();
            let attempts = host.section_attempts;
            assert_eq!(&actual, host.replies.last().unwrap());
            host.section_fault = None;
            host.fault_site = None;
            for _ in 0..3 {
                assert!(cursor.step(&mut host).is_err());
            }
            assert_eq!(host.section_attempts, attempts);
            assert_eq!(cursor.state.sections[site.index()].as_ref(), Some(&actual));
            assert_eq!(&cursor.state.parent, before.parent());
            assert_eq!(cursor.state.writes(), before.writes());
        }
    }
}

#[test]
fn layout_valid_debug_aliases_and_reused_capabilities_fail_complete_ownership_checks() {
    let faults: [fn(&SectionCall, &mut InitializedSection); 6] = [
        |c, r| {
            *r = InitializedSection::from_debug_info(
                &c.section_request(),
                Some(&Owner {
                    token: c.checkpoint().parent().token + 1100,
                    data: vec![0; 32],
                }),
                r.capability,
            )
            .unwrap()
        },
        |c, r| {
            *r = InitializedSection::from_debug_info(
                &c.section_request(),
                Some(&Owner {
                    token: c.checkpoint().caller_frame().token + 800,
                    data: vec![0; 32],
                }),
                r.capability,
            )
            .unwrap()
        },
        |c, r| {
            *r = InitializedSection::from_debug_info(
                &c.section_request(),
                Some(&Owner {
                    token: 0x3000_0000,
                    data: vec![0; 32],
                }),
                r.capability,
            )
            .unwrap()
        },
        |c, r| {
            *r = InitializedSection::from_debug_info(
                &c.section_request(),
                Some(&Owner {
                    token: 0x5000_0000,
                    data: vec![0; 32],
                }),
                r.capability,
            )
            .unwrap()
        },
        |c, r| {
            *r = InitializedSection::from_debug_info(
                &c.section_request(),
                Some(&Owner {
                    token: 0x6000_0000,
                    data: vec![0; 32],
                }),
                r.capability,
            )
            .unwrap()
        },
        |_, r| r.capability = 100,
    ];
    for fault in faults {
        let mut cursor = Construction::new(incoming()).unwrap();
        let mut host = SyntheticHost::default();
        reach(
            &mut cursor,
            &mut host,
            Call::InitializeSection(SectionSite::Resource),
        );
        host.fault_site = Some(SectionSite::Resource);
        host.section_fault = Some(fault);
        assert!(cursor.step(&mut host).is_err());
        // Layout alone accepts these; complete retained owners and per-section
        // capabilities reject them at the source cursor boundary.
        cursor.state.sections[2]
            .as_ref()
            .unwrap()
            .validate(&host.calls.last().unwrap().section_request())
            .unwrap();
        assert!(cursor.step(&mut host).is_err());
        assert_eq!(host.section_effects, 3);
    }
}

#[test]
fn earlier_section_reply_and_foreign_section_region_cannot_service_a_later_call() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    reach(
        &mut cursor,
        &mut host,
        Call::InitializeSection(SectionSite::List),
    );
    host.swapped = Some(host.replies[0].clone());
    host.fault_site = Some(SectionSite::List);
    assert!(cursor.step(&mut host).is_err());
    assert!(cursor.step(&mut host).is_err());
    assert_eq!(host.section_effects, 2);
    assert_eq!(cursor.state.sections[1].as_ref(), Some(&host.replies[0]));
}

#[test]
fn nullable_debug_is_an_actual_valid_return_for_each_distinct_section() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost {
        null_debug: [true; 3],
        ..SyntheticHost::default()
    };
    let cut = finish(&mut cursor, &mut host);
    for section in cut.sections().iter().flatten() {
        assert!(section.debug_info.is_none());
        assert_ne!(section.capability, 0);
        let mut expected = vec![0; 24];
        expected[4..8].fill(0xff);
        assert_eq!(section.region.data, expected);
    }
    assert_eq!(word(&cut.parent().data[256..260]), 1);
    assert_eq!((host.list_effects, host.section_effects), (1, 3));
}

#[test]
fn all_frame_and_argument_bounds_are_checked_before_effects_and_return_the_actual_incoming() {
    let input = incoming();
    let b = input.factory.caller().token;
    let original = input.frames;
    let mut cases = Vec::new();
    for bad in [0, 19, b + 19, b + 1025, 1_u64 << 32, u64::MAX] {
        let mut f = original;
        f.collection = bad;
        cases.push(f);
    }
    for bad in [0, 15, b + 15, b + 1025, 1_u64 << 32, u64::MAX] {
        let mut f = original;
        f.list = bad;
        cases.push(f);
        let mut f = original;
        f.section_base = bad;
        cases.push(f);
    }
    for bad in [0, b - 1, b + 1021, (1_u64 << 32) - 3, u64::MAX - 1] {
        let mut f = original;
        f.resource_argument = bad;
        cases.push(f);
    }
    let mut f = original;
    f.list = f.collection;
    cases.push(f); // EH alias
    let mut f = original;
    f.section_base = f.list;
    cases.push(f);
    let mut f = original;
    f.resource_argument = f.collection - 19;
    cases.push(f);
    let old = input.factory.request().incoming().frames;
    for address in [
        old.factory - 16,
        old.factory - 4,
        old.derived - 16,
        old.base - 16,
        old.base - 4,
        old.refcount - 2,
        old.child - 20,
        old.child - 14,
        old.child - 4,
        old.base72 - 16,
        old.base72 - 4,
        old.base72 + 8,
    ] {
        let mut f = original;
        f.resource_argument = address;
        cases.push(f);
    }
    for frames in cases {
        let factory = input.factory.clone();
        let pointer = factory.parent().data.as_ptr();
        let error = Construction::new(Incoming { factory, frames }).unwrap_err();
        assert!(matches!(error.stop, Stop::Unsupported(_)));
        assert_eq!(error.incoming.frames, frames);
        assert_eq!(error.incoming.factory.parent().data.as_ptr(), pointer);
        assert!(
            error
                .incoming
                .factory
                .request()
                .same_call(input.factory.request())
        );
    }
    // New explicit operands can occupy a gap inside a nominal old machine
    // frame. Only simultaneously live source operand ranges are excluded.
    let mut frames = original;
    frames.collection = b + 72;
    Construction::new(Incoming {
        factory: input.factory.clone(),
        frames,
    })
    .unwrap();
    // An exact end-exclusive resource DWORD is inside the caller owner.
    let mut frames = original;
    frames.resource_argument = b + 1020;
    Construction::new(Incoming {
        factory: input.factory,
        frames,
    })
    .unwrap();
}

#[test]
fn factory_checkpoint_before_the_last_eh_byte_store_is_not_collection_entry() {
    let factory = factory_cut(false);
    factory.validate().unwrap();
    assert!(factory.base72().is_some());
    assert!(factory.tree(factory::TreeSite::Derived).is_some());
    let b = factory.caller().token;
    let error = Construction::new(Incoming {
        factory,
        frames: Frames {
            collection: b + 384,
            list: b + 448,
            section_base: b + 512,
            resource_argument: b + 560,
        },
    })
    .unwrap_err();
    assert!(matches!(error.stop, Stop::Unsupported(_)));
    assert_ne!(
        error.incoming.factory.writes().last().unwrap().instruction,
        0x5d5875
    );
}

#[test]
fn synchronization_and_before_effect_errors_keep_the_exact_pending_requests() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    for site in SITES {
        reach(&mut cursor, &mut host, Call::InitializeSection(site));
        host.fail_sync = true;
        let attempts = host.section_attempts;
        assert!(cursor.step(&mut host).is_err());
        assert_eq!(host.section_attempts, attempts);
        let call = cursor.state.section_calls[site.index()]
            .as_ref()
            .unwrap()
            .clone();
        let before = cursor.checkpoint().unwrap();
        host.fail_section = Some(site);
        assert!(cursor.step(&mut host).is_err());
        assert!(call.same_call(cursor.state.section_calls[site.index()].as_ref().unwrap()));
        assert!(call.same_call(host.calls.last().unwrap()));
        assert_eq!(cursor.state.writes(), before.writes());
        cursor.step(&mut host).unwrap();
        assert!(call.same_call(host.calls.last().unwrap()));
    }
    finish(&mut cursor, &mut host);
    assert_eq!(host.section_effects, 3);
    assert_eq!(host.section_attempts, 6);

    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    reach(&mut cursor, &mut host, Call::AllocateList);
    host.fail_sync = true;
    assert!(cursor.step(&mut host).is_err());
    assert_eq!(host.list_attempts, 0);
    let call = cursor.state.list_call.as_ref().unwrap().clone();
    host.fail_list = true;
    assert!(cursor.step(&mut host).is_err());
    assert!(call.same_call(host.lists.last().unwrap()));
    cursor.step(&mut host).unwrap();
    assert!(call.same_call(host.lists.last().unwrap()));
    finish(&mut cursor, &mut host);
    assert_eq!((host.list_attempts, host.list_effects), (2, 1));
}

#[test]
fn host_owned_effects_after_error_are_acknowledged_without_duplicate_allocation_or_initialization()
{
    for site in SITES {
        let mut cursor = Construction::new(incoming()).unwrap();
        let mut host = SyntheticHost {
            list_after_effect: true,
            section_after_effect: Some(site),
            ..SyntheticHost::default()
        };
        let mut stops = 0;
        let mut reached = false;
        for _ in 0..96 {
            match cursor.step(&mut host) {
                Err(Stop::Unavailable(_)) => {
                    stops += 1;
                    cursor.checkpoint().unwrap();
                }
                Ok(Some(Cut::BeforeResource)) => {
                    reached = true;
                    break;
                }
                Ok(None) => {}
                other => panic!("unexpected synthetic result: {other:?}"),
            }
        }
        assert!(reached);
        assert_eq!(stops, 2);
        assert_eq!((host.list_effects, host.section_effects), (1, 3));
        assert_eq!((host.list_attempts, host.section_attempts), (2, 4));
        assert!(host.lists[0].same_call(&host.lists[1]));
        let repeats: Vec<_> = host.calls.iter().filter(|c| c.site() == site).collect();
        assert_eq!(repeats.len(), 2);
        assert!(repeats[0].same_call(repeats[1]));
    }
}

#[test]
fn cut_sync_failure_retries_only_the_cut_and_stale_or_foreign_sync_cannot_rewind() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    reach(&mut cursor, &mut host, Call::AllocateList);
    let old = cursor.checkpoint().unwrap();
    reach(
        &mut cursor,
        &mut host,
        Call::Resource {
            instruction: 0x5d9339,
            import_slot: 0x666238,
            arguments: [0, 0, 0, 0],
        },
    );
    host.fail_sync = true;
    assert!(cursor.step(&mut host).is_err());
    assert!(!cursor.stopped);
    assert_eq!(cursor.step(&mut host).unwrap(), Some(Cut::BeforeResource));
    let events = host.events.len();
    assert!(cursor.step(&mut host).is_err());
    assert_eq!(host.events.len(), events);
    old.validate().unwrap(); // Historical evidence is valid, but cannot reset live state.
    assert!(host.synchronize_checkpoint(&old).is_err());
    let mut other = Construction::new(Incoming {
        factory: cursor.request().factory().clone(),
        frames: cursor.request().frames(),
    })
    .unwrap();
    let mut other_host = SyntheticHost::default();
    let foreign = finish(&mut other, &mut other_host);
    assert_eq!(foreign.parent(), cursor.checkpoint().unwrap().parent());
    assert!(host.synchronize_checkpoint(&foreign).is_err());
    assert_eq!((host.list_effects, host.section_effects), (1, 3));
}

#[test]
fn getter_and_caller_argument_values_are_read_from_the_source_images() {
    let cursor = Construction::new(incoming()).unwrap();
    let mut parent = cursor.state.parent.clone();
    let mut caller = cursor.state.caller.clone();
    let getter = Store {
        place: Place::Parent(256),
        value: Value::Getter,
        pc: 0x5d9492,
        byte: false,
    };
    for value in [0_u32, 1, 2, 0x8000_0000, u32::MAX] {
        put(&mut parent, 248, value);
        let (bytes, pc) = getter
            .bytes(
                &parent,
                &caller,
                None,
                cursor.request(),
                &[None, None, None],
            )
            .unwrap();
        assert_eq!(word(&bytes), u32::from(value != 0));
        assert_eq!(pc, if value == 0 { 0x5d949b } else { 0x5d9492 });
    }
    let at = Slot::Argument.offset(cursor.request()).unwrap();
    let argument = Store {
        place: Place::Parent(264),
        value: Value::Argument,
        pc: 0x5d9336,
        byte: false,
    };
    for value in [0_u32, 1, 0x7a31_9113, u32::MAX] {
        put(&mut caller, at, value);
        let (bytes, _) = argument
            .bytes(
                &parent,
                &caller,
                None,
                cursor.request(),
                &[None, None, None],
            )
            .unwrap();
        assert_eq!(word(&bytes), value);
    }
    // Pure operand evaluation above is not a way to adopt constructor state.
    assert_eq!(cursor.state.position, 0);
}

#[test]
fn exact_native_end_and_retained_return_budget_boundaries() {
    assert!(reserve_return_budget(MAX_ALLOCATION_BYTES - 120).is_ok());
    assert!(reserve_return_budget(MAX_ALLOCATION_BYTES - 119).is_err());
    assert!(reserve_return_budget(usize::MAX).is_err());
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost {
        list_override: Some(Allocated::Bytes {
            token: (1_u64 << 32) - 24,
            data: vec![0x71; 24],
        }),
        ..SyntheticHost::default()
    };
    finish(&mut cursor, &mut host).validate().unwrap();
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost {
        fault_site: Some(SectionSite::Resource),
        section_fault: Some(|call, reply| {
            *reply = InitializedSection::from_debug_info(
                &call.section_request(),
                Some(&Owner {
                    token: (1_u64 << 32) - 32,
                    data: vec![0x71; 32],
                }),
                reply.capability,
            )
            .unwrap();
        }),
        ..SyntheticHost::default()
    };
    finish(&mut cursor, &mut host).validate().unwrap();
}

#[test]
fn complete_checkpoint_replay_rejects_history_image_return_and_call_tampering() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    let valid = finish(&mut cursor, &mut host);
    for change in 0..19 {
        let mut cut = valid.clone();
        match change {
            0 => cut.parent.data[1200] ^= 1,
            1 => cut.caller.data[1] ^= 1,
            2 => cut.list.as_mut().unwrap().data[23] ^= 1,
            3 => cut.initial_list.as_mut().unwrap().data[23] ^= 1,
            4 => cut.history[0].before[0] ^= 1,
            5 => cut.history[0].after[0] ^= 1,
            6 => cut.history[0].instruction ^= 1,
            7 => cut.history[0].offset += 1,
            8 => cut.history[0].owner += 1,
            9 => cut.history[0].child_return = true,
            10 => {
                cut.history[0].before.pop();
            }
            11 => cut.position -= 1,
            12 => cut.position += 1,
            13 => cut.sections[0].as_mut().unwrap().capability += 19,
            14 => {
                cut.sections[2]
                    .as_mut()
                    .unwrap()
                    .debug_info
                    .as_mut()
                    .unwrap()
                    .data[4] ^= 1
            }
            15 => cut.section_calls.swap(0, 2),
            16 => cut.sections.swap(0, 2),
            17 => cut.list_call = None,
            _ => cut.section_calls[1] = None,
        }
        assert!(
            cut.validate().is_err(),
            "complete checkpoint tamper {change}"
        );
    }
    let mut cursor = Construction::new(incoming()).unwrap();
    let mut host = SyntheticHost::default();
    reach(
        &mut cursor,
        &mut host,
        Call::InitializeSection(SectionSite::Collection),
    );
    let before = cursor.checkpoint().unwrap();
    for change in 0..4 {
        let mut cut = before.clone();
        match change {
            0 => cut.history[6].after[0] = 1, // unexecuted import history
            1 => cut.position = OPS.len(),    // no actual future returns
            2 => cut.history[6].owner = cut.parent.token,
            _ => cut.list_returned = true,
        }
        assert!(cut.validate().is_err(), "future checkpoint tamper {change}");
    }
    cursor.step(&mut host).unwrap();
    let retained = cursor.state.sections[0].clone();
    cursor.state.caller.data[1] ^= 1;
    assert!(cursor.step(&mut host).is_err());
    assert_eq!(cursor.state.sections[0], retained);
    assert_eq!(host.section_effects, 1);
}

#[test]
fn preallocated_write_buffers_survive_all_successful_external_effects() {
    let mut cursor = Construction::new(incoming()).unwrap();
    let capacity = cursor.state.history.capacity();
    let pointers: Vec<_> = cursor
        .state
        .history
        .iter()
        .map(|w| (w.before.as_ptr(), w.after.as_ptr()))
        .collect();
    let list_storage = cursor.list_buffer.as_ref().unwrap().data.as_ptr();
    let mut host = SyntheticHost::default();
    finish(&mut cursor, &mut host);
    assert_eq!(cursor.state.history.capacity(), capacity);
    assert_eq!(
        cursor
            .state
            .history
            .iter()
            .map(|w| (w.before.as_ptr(), w.after.as_ptr()))
            .collect::<Vec<_>>(),
        pointers
    );
    assert_eq!(
        cursor.state.list.as_ref().unwrap().data.as_ptr(),
        list_storage
    );
}
