use super::{Construction, Cut, State};
use serde_json::{Value, json};

fn finish(mut construction: Construction) -> State {
    while construction.step().unwrap().is_some() {}
    construction.into_state().unwrap()
}

#[test]
fn ordered_stores_preserve_unwritten_source_bytes() {
    let source = std::array::from_fn(|offset| (offset as u8).wrapping_mul(17));
    let mut construction = Construction::new(source);
    let mut expected = source;
    for (index, (instruction, value, cut)) in [
        (0x60791e, 640_u32, Cut::WidthStored),
        (0x607928, 480, Cut::HeightStored),
        (0x607932, 16, Cut::DepthStored),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = index * 4;
        let before: [u8; 4] = expected[offset..offset + 4].try_into().unwrap();
        assert_eq!(construction.step().unwrap(), Some(cut));
        expected[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert_eq!(construction.bytes(), &expected);
        assert_eq!(construction.source(), &source);
        let write = &construction.writes()[index];
        assert_eq!(write.instruction, instruction);
        assert_eq!(write.address, 0x6ff0d8 + offset as u32);
        assert_eq!(write.before, before);
        assert_eq!(write.after, value.to_le_bytes());
    }
    assert!(construction.clone().into_state().is_err());
    assert_eq!(construction.step().unwrap(), Some(Cut::Returned));
    assert_eq!(construction.step().unwrap(), None);
    assert_eq!(construction.writes().len(), 3);
    construction.into_state().unwrap().validate().unwrap();
}

#[test]
fn every_cut_roundtrips_and_continues_for_all_initial_byte_values() {
    for beginning in 0_u8..=255 {
        let source = std::array::from_fn(|offset| beginning.wrapping_add((offset * 17) as u8));
        let expected = finish(Construction::new(source));
        let mut construction = Construction::new(source);
        for cut in 0..5 {
            let restored: Construction =
                serde_json::from_slice(&serde_json::to_vec(&construction).unwrap()).unwrap();
            assert_eq!(restored, construction);
            assert_eq!(finish(restored), expected);
            if cut < 4 {
                assert!(construction.step().unwrap().is_some());
            }
        }
    }
}

#[test]
fn retained_state_rejects_changed_bytes_order_addresses_and_cuts() {
    let complete = finish(Construction::new([0xa5; 12]));
    let original = serde_json::to_value(complete.construction()).unwrap();
    let changes = [
        ("/bytes/0", json!(0)),
        ("/source/0", json!(0)),
        ("/writes/0/before/0", json!(0)),
        ("/writes/0/after/0", json!(0)),
        ("/writes/0/instruction", json!(0x607928)),
        ("/writes/0/address", json!(0x6ff0dc)),
        ("/cut", json!("DepthStored")),
    ];
    for (pointer, replacement) in changes {
        let mut damaged = original.clone();
        *damaged.pointer_mut(pointer).unwrap() = replacement;
        assert!(serde_json::from_value::<Construction>(damaged).is_err());
    }
    let mut reversed = original.clone();
    reversed["writes"].as_array_mut().unwrap().reverse();
    assert!(serde_json::from_value::<Construction>(reversed).is_err());
    let mut extra = original.clone();
    extra["writes"]
        .as_array_mut()
        .unwrap()
        .push(original["writes"][0].clone());
    assert!(serde_json::from_value::<Construction>(extra).is_err());
    let mut unknown = original.clone();
    unknown["initialized_windows"] = Value::Bool(true);
    assert!(serde_json::from_value::<Construction>(unknown).is_err());
    let mut unknown_write = original;
    unknown_write["writes"][0]["ignored"] = Value::Bool(true);
    assert!(serde_json::from_value::<Construction>(unknown_write).is_err());
}

#[test]
fn completed_state_cannot_be_restored_from_an_earlier_cut() {
    let mut construction = Construction::new([0x3c; 12]);
    for _ in 0..4 {
        assert!(construction.clone().into_state().is_err());
        assert!(serde_json::from_value::<State>(json!({"construction": construction})).is_err());
        construction.step().unwrap();
    }
    let state = construction.into_state().unwrap();
    let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(restored, state);
    restored.validate().unwrap();
}

#[test]
fn separate_initialization_epochs_retain_independent_sources_and_cuts() {
    let mut first = Construction::new([0x19; 12]);
    let mut second = Construction::new([0xe7; 12]);
    first.step().unwrap();
    assert_eq!(second.cut(), Cut::Entry);
    assert_eq!(second.bytes(), &[0xe7; 12]);
    second.step().unwrap();
    second.step().unwrap();
    assert_eq!(first.cut(), Cut::WidthStored);
    assert_eq!(&first.bytes()[4..], &[0x19; 8]);
    assert_eq!(finish(first).construction().source(), &[0x19; 12]);
    assert_eq!(finish(second).construction().source(), &[0xe7; 12]);
    let fresh = Construction::new([0; 12]);
    assert_eq!(fresh.cut(), Cut::Entry);
    assert!(fresh.writes().is_empty());
}

#[test]
fn matching_initial_dimensions_do_not_skip_original_stores_or_return() {
    let source = [640_u32, 480, 16]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let initial = Construction::new(source);
    assert!(initial.clone().into_state().is_err());
    let state = finish(initial);
    assert_eq!(state.construction().writes().len(), 3);
    for write in state.construction().writes() {
        assert_eq!(write.before, write.after);
    }
}
