use crate::native_game::simulator_binding::{Construction, Cut, State};
use std::sync::OnceLock;

fn construction() -> Construction {
    static SOURCE: OnceLock<crate::native_game::object_registry::State> = OnceLock::new();
    let source = SOURCE.get_or_init(|| {
        let mut execution = super::construction(0x9d);
        super::reach(&mut execution, &mut super::host(), super::Cut::Returned);
        execution.into_state().unwrap()
    });
    Construction::new(source.clone()).unwrap()
}

fn reach(execution: &mut Construction, cut: Cut) {
    for _ in 0..3 {
        if execution.cut() == cut {
            return;
        }
        execution.step().unwrap();
    }
    panic!("Simulator binding prefix did not reach {cut:?}");
}

#[test]
fn binding_publishes_only_current_registry_to_game_before_mutating_simulator() {
    let mut execution = construction();
    let source = execution.source().clone();
    let original_game = execution.game().clone();
    let original_simulator = execution.simulator().clone();
    let registry = execution.registry_owner().token as u32;
    assert_eq!(execution.step().unwrap(), Some(Cut::BindingEntry));
    let mut expected = original_game.clone();
    expected.data[0x14..0x18].copy_from_slice(&registry.to_le_bytes());
    assert_eq!(execution.game(), &expected);
    assert_eq!(execution.simulator(), &original_simulator);
    assert_eq!(execution.source(), &source);
    assert_eq!(execution.writes().len(), 1);
    let event = &execution.writes()[0];
    assert_eq!(
        (event.owner, event.offset, event.instruction),
        (expected.token, 0x14, 0x415cfd)
    );
    assert_eq!(event.before, original_game.data[0x14..0x18]);
    assert_eq!(event.after, registry.to_le_bytes());
}

#[test]
fn binding_retains_all_stores_including_same_value_zeroes_and_untouched_bytes() {
    let mut execution = construction();
    let source = execution.source().clone();
    let simulator = execution.simulator().clone();
    let registry = execution.registry_owner().token as u32;
    reach(&mut execution, Cut::ProcessLanguageInput);
    assert_eq!(execution.source(), &source);
    assert_eq!(execution.registry(), source.construction().registry());
    assert_eq!(execution.writes().len(), 47);
    let events = &execution.writes()[1..];
    assert_eq!(events.len(), 46);
    assert_eq!((events[0].offset, events[0].instruction), (176, 0x46cec7));
    assert_eq!(events[0].after, registry.to_le_bytes());
    for (index, event) in events[1..33].iter().enumerate() {
        assert_eq!(
            (event.offset, event.instruction),
            (16 + index * 4, 0x46ced4)
        );
        assert_eq!(event.before, [0; 4]);
        assert_eq!(event.after, [0; 4]);
    }
    let mut expected = simulator.data.clone();
    let mut mask = [false; 428];
    for event in events {
        assert_eq!(event.owner, simulator.token);
        let range = event.offset..event.offset + event.after.len();
        assert_eq!(expected[range.clone()], event.before);
        expected[range.clone()].copy_from_slice(&event.after);
        mask[range].fill(true);
    }
    assert_eq!(execution.simulator().data, expected);
    assert_eq!(mask.iter().filter(|written| **written).count(), 152);
    for (offset, written) in mask.iter().enumerate() {
        if !written {
            assert_eq!(execution.simulator().data[offset], simulator.data[offset]);
        }
    }
    for (offset, value) in [
        (16, 1_u16),
        (18, 15),
        (24, 2),
        (26, 0),
        (30, 6),
        (32, 1997),
        (48, 2),
        (52, u16::MAX),
    ] {
        assert_eq!(
            execution.simulator().data[offset..offset + 2],
            value.to_le_bytes()
        );
    }
    assert_eq!(execution.simulator().data[168..172], 50_u32.to_le_bytes());
    assert_eq!(
        execution.source().construction().incoming().comparator.data,
        [0x9d]
    );
}

#[test]
fn binding_semantic_cuts_roundtrip_without_repeating_publication_or_prefix() {
    for (cut, stores) in [
        (Cut::CallerEntry, 0),
        (Cut::BindingEntry, 1),
        (Cut::ProcessLanguageInput, 47),
    ] {
        let mut warm = construction();
        reach(&mut warm, cut);
        assert_eq!(warm.writes().len(), stores);
        let mut cold: Construction =
            serde_json::from_slice(&serde_json::to_vec(&warm).unwrap()).unwrap();
        assert_eq!(cold, warm);
        reach(&mut warm, Cut::ProcessLanguageInput);
        reach(&mut cold, Cut::ProcessLanguageInput);
        assert_eq!(cold, warm);
        assert_eq!(cold.step().unwrap(), None);
        assert_eq!(cold, warm);
        let state = cold.into_state().unwrap();
        state.validate().unwrap();
        assert_eq!(
            serde_json::from_slice::<State>(&serde_json::to_vec(&state).unwrap()).unwrap(),
            state
        );
    }
}

#[test]
fn binding_restore_rejects_changed_owners_stores_preimages_and_cut() {
    let mut execution = construction();
    reach(&mut execution, Cut::ProcessLanguageInput);
    let original = serde_json::to_value(&execution).unwrap();
    for pointer in [
        "/owners/0/token",
        "/owners/0/data/20",
        "/owners/1/token",
        "/owners/1/data/427",
        "/writes/0/owner",
        "/writes/0/offset",
        "/writes/0/instruction",
        "/writes/0/before/0",
        "/writes/0/after/0",
        "/writes/2/before/0",
        "/writes/2/after/0",
        "/writes/46/instruction",
    ] {
        let mut modified = original.clone();
        let value = modified.pointer_mut(pointer).unwrap();
        *value = serde_json::json!(value.as_u64().unwrap() ^ 1);
        assert!(
            serde_json::from_value::<Construction>(modified).is_err(),
            "{pointer}"
        );
    }
    for field in ["owners", "writes"] {
        let mut modified = original.clone();
        modified[field].as_array_mut().unwrap().pop();
        assert!(
            serde_json::from_value::<Construction>(modified).is_err(),
            "{field}"
        );
    }
    let mut modified = original.clone();
    modified["cut"] = serde_json::json!("BindingEntry");
    assert!(serde_json::from_value::<Construction>(modified).is_err());
    let mut modified = original;
    modified["writes"].as_array_mut().unwrap().swap(2, 3);
    assert!(serde_json::from_value::<Construction>(modified).is_err());
}

#[test]
fn binding_restore_requires_validated_predecessor_and_rejects_extra_process_inputs() {
    let original = serde_json::to_value(construction()).unwrap();
    for field in ["source", "owners", "writes", "cut"] {
        let mut modified = original.clone();
        modified.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<Construction>(modified).is_err(),
            "{field}"
        );
    }
    for field in [
        "process_language",
        "expansion",
        "rng",
        "resources",
        "provider",
    ] {
        let mut modified = original.clone();
        modified[field] = serde_json::json!(0);
        assert!(
            serde_json::from_value::<Construction>(modified).is_err(),
            "{field}"
        );
    }
    let mut modified = original.clone();
    modified["source"]["construction"]["cut"] = serde_json::json!("TreeAllocated");
    assert!(serde_json::from_value::<Construction>(modified).is_err());
    let mut modified = original;
    modified["source"]["construction"]["comparator_read"]["byte"] = serde_json::json!(0);
    assert!(serde_json::from_value::<Construction>(modified).is_err());
}

#[test]
fn binding_successor_guards_complete_prior_graph_and_comparator_source() {
    let execution = construction();
    let registry = execution.source().construction();
    for (token, bytes) in [
        (execution.game().token + 111, 1),
        (execution.simulator().token + 427, 1),
        (execution.registry_owner().token + 271, 1),
        (registry.owners()[1].token + 23, 1),
        (registry.incoming().comparator.token, 1),
        (0x6e5528, 4),
        (u64::MAX, 2),
        (0, 1),
    ] {
        assert!(
            execution.guard_span(token, bytes).is_err(),
            "{token:#x}/{bytes}"
        );
    }
    execution.guard_span(0x7c000000, 64).unwrap();
}

#[test]
fn binding_prefix_state_requires_actual_unresolved_language_boundary() {
    for cut in [Cut::CallerEntry, Cut::BindingEntry] {
        let mut execution = construction();
        reach(&mut execution, cut);
        assert!(execution.clone().into_state().is_err());
        assert!(
            serde_json::from_value::<State>(serde_json::json!({"construction": execution}))
                .is_err()
        );
    }
    let mut execution = construction();
    reach(&mut execution, Cut::ProcessLanguageInput);
    let state = execution.into_state().unwrap();
    assert_eq!(state.construction().cut(), Cut::ProcessLanguageInput);
    assert_eq!(
        state.construction().writes().last().unwrap().instruction,
        0x46cf25
    );
    let mut retained = serde_json::to_value(state).unwrap();
    retained["process_language"] = serde_json::json!(0);
    assert!(serde_json::from_value::<State>(retained).is_err());
}
