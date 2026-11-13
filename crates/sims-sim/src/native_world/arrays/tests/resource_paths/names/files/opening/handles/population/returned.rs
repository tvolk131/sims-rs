use super::*;
use crate::native_world::returned;

mod game_publication;
mod startup;

fn incoming(origin: u64) -> returned::Incoming {
    returned::Incoming {
        corners: std::array::from_fn(|index| Owner {
            token: origin + [4, 8, 0, 20][index],
            data: vec![
                0x31 + index as u8,
                0x41 + index as u8,
                0x51 + index as u8,
                0x61 + index as u8,
            ],
        }),
    }
}

fn returned_construction() -> returned::Construction {
    let (mut execution, mut host) = construction();
    reach(&mut execution, &mut host, 99);
    returned::Construction::new(execution.into_state().unwrap(), incoming(0x78000000)).unwrap()
}

fn reach_return(execution: &mut returned::Construction, target: returned::Cut) {
    for _ in 0..30 {
        if execution.cut() == Some(target) {
            return;
        }
        execution.step().unwrap();
    }
    panic!("actual world return cut was not reached");
}

#[test]
fn composed_world_return_preserves_complete_graph_and_native_write_order() {
    let mut execution = returned_construction();
    let source = execution.population().clone();
    let initial = execution.owner().clone();
    let mut cuts = vec![];
    let mut roundtrips = 0;
    for _ in 0..30 {
        if let Some(cut) = execution.step().unwrap() {
            cuts.push(cut);
        }
        let restored: returned::Construction =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(execution, restored);
        assert_eq!(
            execution.registry().nodes[&(execution.owner().token as u32)].handler,
            u32::from_le_bytes(execution.owner().data[..4].try_into().unwrap())
        );
        restored.validate().unwrap();
        execution = restored;
        roundtrips += 1;
        if execution.cut() == Some(returned::Cut::Returned) {
            break;
        }
    }
    assert_eq!(roundtrips, 23);
    assert_eq!(cuts.len(), 11);
    assert_eq!(execution.population(), &source);
    assert_eq!(execution.returned_owner(), Some(initial.token));
    assert_eq!(execution.writes().len(), 16);
    assert_eq!(
        execution
            .writes()
            .iter()
            .map(|write| write.offset)
            .collect::<Vec<_>>(),
        [
            128, 129, 132, 0, 112, 112, 112, 116, 116, 116, 120, 120, 120, 124, 124, 124
        ]
    );
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    assert_eq!(state.active_size(), 64);
    assert_eq!(state.orientation(), 0);
    assert_eq!(
        state.registry().nodes[&(state.owner().token as u32)].handler,
        0x667a94
    );
    assert_eq!(state.bounds(), [0, 63, 0, 63]);
    assert_eq!(&state.owner().data[4..112], &initial.data[4..112]);
    assert_eq!(&state.owner().data[130..132], &initial.data[130..132]);
    assert_eq!(&state.owner().data[..4], &0x667a94_u32.to_le_bytes());
}

#[test]
fn world_return_rejects_a_stale_registry_or_unowned_game_claim() {
    let mut execution = returned_construction();
    reach_return(&mut execution, returned::Cut::Returned);
    let state = execution.clone().into_state().unwrap();
    assert!(crate::native_game::WorldReturned::new(state).is_err());
    let encoded = serde_json::to_value(&execution).unwrap();
    for value in [0x667a88, 0] {
        let mut changed = encoded.clone();
        changed["registry"]["nodes"][execution.owner().token.to_string()]["handler"] = value.into();
        let mut restored: returned::Construction = serde_json::from_value(changed).unwrap();
        assert!(restored.validate().is_err());
        assert!(restored.step().is_err());
        assert!(restored.into_state().is_err());
    }
}

#[test]
fn fixed_return_writes_two_bytes_without_zeroing_padding_or_rotatable_fields() {
    let mut execution = returned_construction();
    let mut expected = execution.owner().clone();
    reach_return(&mut execution, returned::Cut::LightingReturned);
    assert_eq!(execution.owner(), &expected);
    reach_return(&mut execution, returned::Cut::FixedReturned);
    expected.data[128] = 0;
    expected.data[129] = 0;
    assert_eq!(execution.owner(), &expected);
    assert_eq!(execution.writes().len(), 2);
}

#[test]
fn actual_corner_calls_preserve_complete_fourth_bytes_and_return_input_identity() {
    let mut execution = returned_construction();
    let initial = execution.incoming().clone();
    reach_return(&mut execution, returned::Cut::VtableBound);
    assert_eq!(execution.corners()[1].data[0], initial.corners[1].data[0]);
    assert_eq!(
        &execution.corners()[2].data[..2],
        &initial.corners[2].data[..2]
    );
    for (index, coordinates) in [[0, 0, 0], [63, 0, 0], [63, 63, 0], [0, 63, 0]]
        .into_iter()
        .enumerate()
    {
        reach_return(&mut execution, returned::Cut::CornerReturned(index as u8));
        assert_eq!(&execution.corners()[index].data[..3], &coordinates);
        assert_eq!(
            execution.corners()[index].data[3],
            initial.corners[index].data[3]
        );
        assert_eq!(
            execution.corner_returns()[index],
            initial.corners[index].token
        );
    }
}

#[test]
fn malformed_corner_inputs_and_wrong_frame_layout_cannot_begin_return() {
    let execution = returned_construction();
    for index in 0..4 {
        let mut frame = incoming(0x78000000);
        frame.corners[index].data.pop();
        assert!(returned::Construction::new(execution.population().clone(), frame).is_err());
    }
    let mut frame = incoming(0x78000000);
    frame.corners[1].token += 1;
    assert!(returned::Construction::new(execution.population().clone(), frame).is_err());
}

#[test]
fn incoming_corner_frame_cannot_alias_retained_world_or_resource_owners() {
    let execution = returned_construction();
    let population = execution.population();
    for token in [
        execution.owner().token,
        population.resources().token,
        population.backings()[0].token,
        population.owners()[0].token,
        population.handle().owner().token,
    ] {
        assert!(returned::Construction::new(population.clone(), incoming(token)).is_err());
    }
}

#[test]
fn incomplete_fixed_and_rotatable_returns_cannot_publish_final_state() {
    for target in [
        returned::Cut::ResourceLoadReturned,
        returned::Cut::FixedReturned,
        returned::Cut::VtableBound,
        returned::Cut::CornerReturned(3),
        returned::Cut::ReturnInstruction,
    ] {
        let mut execution = returned_construction();
        reach_return(&mut execution, target);
        assert!(execution.into_state().is_err());
    }
}

#[test]
fn restored_return_rejects_modified_owner_fields_and_untouched_padding() {
    let mut execution = returned_construction();
    reach_return(&mut execution, returned::Cut::Returned);
    let value = serde_json::to_value(&execution).unwrap();
    for offset in [0, 16, 112, 128, 130, 132] {
        let mut broken = value.clone();
        broken["owner"]["data"][offset] = serde_json::json!(255);
        let broken: returned::Construction = serde_json::from_value(broken).unwrap();
        assert!(broken.validate().is_err());
    }
}

#[test]
fn restored_return_rejects_changed_corner_padding_or_call_identity() {
    let mut execution = returned_construction();
    reach_return(&mut execution, returned::Cut::CornerReturned(3));
    let value = serde_json::to_value(&execution).unwrap();
    let mut broken = value.clone();
    broken["corners"][0]["data"][3] = serde_json::json!(255);
    let broken: returned::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
    let mut broken = value;
    broken["corner_returns"][1] = serde_json::json!(0);
    let broken: returned::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
}

#[test]
fn restored_return_requires_unchanged_actual_write_history_including_same_value_stores() {
    let mut execution = returned_construction();
    reach_return(&mut execution, returned::Cut::Returned);
    let value = serde_json::to_value(&execution).unwrap();
    let mut broken = value.clone();
    broken["writes"][5]["before"][0] = serde_json::json!(255);
    let broken: returned::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
    let mut broken = value;
    broken["writes"].as_array_mut().unwrap().remove(5);
    let broken: returned::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
}

#[test]
fn restored_return_cannot_skip_phases_or_relabel_an_unreturned_state() {
    let execution = returned_construction();
    let value = serde_json::to_value(&execution).unwrap();
    for cursor in [6, 23, 255] {
        let mut broken = value.clone();
        broken["cursor"] = serde_json::json!(cursor);
        let broken: returned::Construction = serde_json::from_value(broken).unwrap();
        assert!(broken.validate().is_err());
    }
    let broken: returned::State =
        serde_json::from_value(serde_json::json!({"execution": value})).unwrap();
    assert!(broken.validate().is_err());
}
