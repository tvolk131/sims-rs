use super::*;
use native::{NumericReply as P, NumericRequest as R, Reply, Request, WindowRequest as W};

fn counted(bytes: &[u8]) -> CountedBinding {
    CountedBinding {
        bytes: bytes.to_vec(),
        capacity: Some(256),
        profile: Some(StorageProfile::CompleteCollection191Inline256Observed20261025),
    }
}
fn backend() -> State {
    State::bind(Binding {
        fields: native::Fields {
            owner: 100,
            state: Some(101),
            label: Some(102),
        },
        application_family: ApplicationFamily::KnownNull,
        counted_text: [(101, counted(b"explicit new CPState"))].into(),
        label_text: [(102, b"explicit new label".to_vec())].into(),
    })
    .unwrap()
}
fn cold(state: &mut State) {
    state.validate_owned().unwrap();
    let bytes = serde_json::to_vec(state).unwrap();
    *state = serde_json::from_slice(&bytes).unwrap();
    state.validate_owned().unwrap();
    assert_eq!(bytes, serde_json::to_vec(state).unwrap());
}
fn respond(state: &mut State, request: &Request) -> Result<()> {
    state.expect_service(request)?;
    let reply = match request {
        Request::Numeric(R::LotQuery) => Reply::Numeric(P::Boolean(true)),
        Request::Numeric(R::ApplicationFamily) => {
            Reply::Numeric(P::Owner(application_family(state.application_family)?))
        }
        Request::Numeric(R::Format { value }) => Reply::Numeric(P::Bytes(decimal(*value))),
        _ => state.text_service(request)?,
    };
    state.retain_reply(request, reply)
}

#[test]
fn historical_view_and_application_unknown_null_are_distinct() {
    let value = serde_json::to_value(super::super::View::default()).unwrap();
    assert!(value.get("family_window").is_none());
    assert!(
        serde_json::from_value::<super::super::View>(value)
            .unwrap()
            .family_window
            .is_none()
    );
    let values = [
        ApplicationFamily::Unknown,
        ApplicationFamily::KnownNull,
        ApplicationFamily::Family(0),
    ];
    let encoded: Vec<_> = values
        .iter()
        .map(|v| serde_json::to_vec(v).unwrap())
        .collect();
    assert_ne!(encoded[0], encoded[1]);
    assert_ne!(encoded[1], encoded[2]);
    for (v, b) in values.iter().zip(encoded) {
        assert_eq!(*v, serde_json::from_slice(&b).unwrap());
    }
}
#[test]
fn argument_gate_is_low_byte_and_requires_no_text_fields() {
    let mut state = State::bind(Binding {
        fields: native::Fields {
            owner: 7,
            state: None,
            label: Some(0),
        },
        application_family: ApplicationFamily::Unknown,
        counted_text: BTreeMap::new(),
        label_text: BTreeMap::new(),
    })
    .unwrap();
    state.begin(3, 0x100).unwrap();
    assert_eq!(state.call.as_ref().unwrap().native.owner(), 7);
    assert_eq!(state.call.as_ref().unwrap().native.raw_argument(), 0x100);
    cold(&mut state);
    assert_eq!(state.step().unwrap(), Progress::Pending);
    cold(&mut state);
    assert_eq!(state.step().unwrap(), Progress::Complete);
    state.validate_owned().unwrap();
    assert!(state.counted.is_empty() && state.destroyed.is_empty());
    state.begin(3, 1).unwrap();
    assert!(state.step().is_err());
}
#[test]
fn changed_and_equal_callbacks_retain_real_temporaries_and_exact_ready_cuts() {
    let mut state = backend();
    for expected in [vec![2, 1], vec![1]] {
        state.begin(9, 1).unwrap();
        let mut services = 0;
        for _ in 0..100 {
            cold(&mut state);
            match state.step().unwrap() {
                Progress::Complete => break,
                Progress::Pending => (),
                Progress::Service(request) => {
                    cold(&mut state);
                    respond(&mut state, &request).unwrap();
                    let ready = serde_json::to_vec(&state).unwrap();
                    assert!(respond(&mut state, &request).is_err());
                    assert_eq!(ready, serde_json::to_vec(&state).unwrap());
                    cold(&mut state);
                    services += 1;
                }
            }
        }
        assert!(!state.pending() && services > 0);
        assert_eq!(state.label(102), Some(b"0".as_slice()));
        assert_eq!(state.destruction_order(), expected);
        assert!(state.temporaries.is_empty());
        assert_eq!(state.buffers.len(), 1);
    }
}
#[test]
fn temporary_copies_the_returned_buffer_even_after_current_pointer_changes() {
    let mut state = backend();
    state.begin(1, 1).unwrap();
    for _ in 0..60 {
        let progress = state.step().unwrap();
        let Progress::Service(request) = progress else {
            continue;
        };
        if let Request::Window(W::Temporary { source }) = &request {
            let source = *source;
            let old = state.buffers[&source.buffer].bytes.clone();
            state
                .replace_counted_storage(101, counted(b"later replacement"))
                .unwrap();
            assert_ne!(source.buffer, state.counted[&101].buffer);
            cold(&mut state);
            respond(&mut state, &request).unwrap();
            assert_eq!(state.temporaries[&1], old);
            assert_eq!(state.buffers[&source.buffer].bytes, old);
            return;
        }
        respond(&mut state, &request).unwrap();
    }
    panic!("did not reach actual temporary constructor");
}
#[test]
fn known_null_label_fault_is_late_and_keeps_counted_publication() {
    let mut state = backend();
    state.fields.label = Some(0);
    state.begin(1, 1).unwrap();
    for _ in 0..60 {
        let Progress::Service(request) = state.step().unwrap() else {
            continue;
        };
        if matches!(request, Request::Window(W::Equal { widget: 0, .. })) {
            let before = serde_json::to_vec(&state).unwrap();
            assert!(respond(&mut state, &request).is_err());
            assert_eq!(before, serde_json::to_vec(&state).unwrap());
            assert_eq!(state.buffers[&state.counted[&101].buffer].bytes, b"0");
            assert_eq!(state.created, 1);
            assert!(state.destroyed.is_empty());
            cold(&mut state);
            return;
        }
        respond(&mut state, &request).unwrap();
    }
    panic!("did not reach late null label read");
}
#[test]
fn forged_ready_request_and_buffer_topology_fail_validation() {
    let mut state = backend();
    state.begin(1, 1).unwrap();
    state.step().unwrap();
    let good = state.clone();
    state.call.as_mut().unwrap().ready_reply = Some(Reply::Numeric(P::Done));
    assert!(state.validate_owned().is_err());
    state = good.clone();
    state.call.as_mut().unwrap().pending_request = None;
    assert!(state.validate_owned().is_err());
    state = good.clone();
    state.buffers.get_mut(&1).unwrap().state = 999;
    assert!(state.validate_owned().is_err());
    state = good;
    state.temporaries.insert(1, vec![]);
    assert!(state.validate_owned().is_err());
}
#[test]
fn runtime_lot_query_preserves_zero_early_and_unknown_laziness() {
    let mut world = World::default();
    assert!(lot_query(&world).is_err());
    world.globals.insert(10, 0);
    assert!(lot_query(&world).unwrap());
    world.globals.insert(10, 5);
    assert!(lot_query(&world).is_err());
    world.person = Some(Default::default());
    assert!(!lot_query(&world).unwrap());
    world.person.as_mut().unwrap().lot_zoning.insert(5, 1);
    assert!(lot_query(&world).unwrap());
    world.person.as_mut().unwrap().lot_zoning.insert(5, 2);
    assert!(!lot_query(&world).unwrap());
}
#[test]
fn runtime_guid_lookup_uses_person_order_and_stops_before_unread_tail() {
    let mut world = World::default();
    assert_eq!(person(&world, u32::MAX).unwrap(), None);
    world
        .image_cache
        .entries
        .push(sims_vm::image_cache::Entry::new(123));
    assert!(person(&world, 0).is_err());
    let mut people = sims_vm::person::WorldState::default();
    people.person_order = vec![9, 7, 404];
    people.resource_instances.insert(123, vec![7]);
    world.person = Some(people);
    for id in [7, 9] {
        world.objects.insert(
            id,
            sims_vm::Object {
                definition: Some(sims_vm::Definition {
                    source: "explicit-test".into(),
                    object: 1,
                    guid: 123,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
    }
    assert_eq!(person(&world, 0).unwrap(), Some(person_handle(9)));
    world.person.as_mut().unwrap().person_order = vec![404, 9];
    assert!(person(&world, 0).is_err());
    assert_eq!(person(&world, 99).unwrap(), None);
}
#[test]
fn runtime_family_lookup_is_ordered_signed_and_lazy() {
    let mut world = World::default();
    world.person = Some(Default::default());
    assert!(family(&world, -1).is_err());
    let people = world.person.as_mut().unwrap();
    people.family_order = Some(vec![-1, 33]);
    people.families.insert(-1, vec![]);
    assert_eq!(family(&world, -1).unwrap(), Some(family_handle(-1)));
    assert!(family(&world, 0).is_err());
    world.person.as_mut().unwrap().family_order = Some(vec![-1]);
    assert_eq!(family(&world, 0).unwrap(), None);
    assert_eq!(family_id(family_handle(-1)).unwrap(), -1);
}
#[test]
fn executing_friends_keeps_clean_lazy_and_dirty_empty_projection() {
    let mut world = World::default();
    let mut people = sims_vm::person::WorldState::default();
    people.family_metadata.insert(
        5,
        sims_vm::family::Metadata {
            flags: 0,
            dirty: false,
            friend_count: Some(-17),
            member_guids: None,
        },
    );
    world.person = Some(people);
    assert_eq!(friends(&mut world, family_handle(5)).unwrap(), -17);
    world
        .person
        .as_mut()
        .unwrap()
        .family_metadata
        .get_mut(&5)
        .unwrap()
        .dirty = true;
    assert!(friends(&mut world, family_handle(5)).is_err());
    assert!(world.person.as_ref().unwrap().family_metadata[&5].dirty);
    world.person.as_mut().unwrap().neighbor_slots = Some(vec![]);
    assert_eq!(friends(&mut world, family_handle(5)).unwrap(), 0);
    assert!(!world.person.as_ref().unwrap().family_metadata[&5].dirty);
    assert!(world.autonomy_constants.is_none());
    for (value, expected) in [
        (i32::MIN, "-2147483648"),
        (-1, "-1"),
        (0, "0"),
        (i32::MAX, "2147483647"),
    ] {
        assert_eq!(decimal(value), expected.as_bytes());
    }
}

#[test]
fn store_preserves_the_bound_identity_and_unknown_capacity_stops_only_at_store() {
    let mut state = backend();
    let buffer = state.counted_buffer(101).unwrap();
    state.begin(1, 1).unwrap();
    let mut stores = 0;
    for _ in 0..100 {
        match state.step().unwrap() {
            Progress::Complete => break,
            Progress::Pending => (),
            Progress::Service(request) => {
                respond(&mut state, &request).unwrap();
                if matches!(request, Request::Numeric(R::Store { .. })) {
                    stores += 1;
                    assert_eq!(state.counted_buffer(101), Some(buffer));
                }
            }
        }
    }
    assert_eq!(stores, 2);
    assert_eq!(state.result(), Some(0));
    let mut state = backend();
    state.counted.get_mut(&101).unwrap().capacity = None;
    state.counted.get_mut(&101).unwrap().profile = None;
    state.begin(1, 1).unwrap();
    for _ in 0..60 {
        let Progress::Service(request) = state.step().unwrap() else {
            continue;
        };
        if matches!(request, Request::Numeric(R::Store { .. })) {
            let before = serde_json::to_vec(&state).unwrap();
            assert!(respond(&mut state, &request).is_err());
            assert_eq!(before, serde_json::to_vec(&state).unwrap());
            cold(&mut state);
            return;
        }
        respond(&mut state, &request).unwrap();
    }
    panic!("did not reach unknown-capacity Store boundary");
}
#[test]
fn unknown_application_is_read_only_at_its_actual_service() {
    let mut state = backend();
    state.application_family = ApplicationFamily::Unknown;
    state.begin(1, 1).unwrap();
    for _ in 0..20 {
        let Progress::Service(request) = state.step().unwrap() else {
            continue;
        };
        if matches!(request, Request::Numeric(R::ApplicationFamily)) {
            let before = serde_json::to_vec(&state).unwrap();
            assert!(respond(&mut state, &request).is_err());
            assert_eq!(before, serde_json::to_vec(&state).unwrap());
            cold(&mut state);
            return;
        }
        respond(&mut state, &request).unwrap();
    }
    panic!("did not reach unknown application service");
}

#[test]
fn native_return_cut_keeps_old_buffers_until_explicit_finalization() {
    let mut state = backend();
    state.begin(1, 1).unwrap();
    state
        .replace_counted_storage(101, counted(b"new explicit storage"))
        .unwrap();
    assert_eq!(state.buffers.len(), 2);
    for _ in 0..100 {
        let progress = state.step().unwrap();
        if state.call.as_ref().unwrap().native.complete() {
            assert_eq!(progress, Progress::Pending);
            assert!(state.pending());
            assert_eq!(state.buffers.len(), 2);
            cold(&mut state);
            assert_eq!(state.step().unwrap(), Progress::Complete);
            assert_eq!(state.buffers.len(), 1);
            assert!(!state.pending());
            return;
        }
        if let Progress::Service(request) = progress {
            respond(&mut state, &request).unwrap();
        }
    }
    panic!("did not reach native return");
}
