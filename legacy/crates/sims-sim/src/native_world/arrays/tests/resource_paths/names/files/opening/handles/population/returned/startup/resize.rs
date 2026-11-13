use super::*;
use crate::native_world::resize::{self, Cut};

fn incoming(world: &startup::World, requested: u32, publish: bool) -> resize::Incoming {
    resize::Incoming {
        requested,
        current_world: if publish {
            world.owner().token as u32
        } else {
            0
        },
        corners: (publish && requested as i32 != world.active_size()).then(|| {
            std::array::from_fn(|index| Owner {
                token: 0x7a000000 + [8, 4, 0, 36][index],
                data: if index == 3 {
                    requested.to_le_bytes().to_vec()
                } else {
                    vec![0x31, 0x41, 0x51, 0x61 + index as u8]
                },
            })
        }),
    }
}

fn resized(world: startup::World, requested: u32, publish: bool) -> startup::World {
    let context = incoming(&world, requested, publish);
    let mut execution = resize::Execution::new(world, context).unwrap();
    for _ in 0..8 {
        if execution.step().unwrap() == Some(Cut::Returned) {
            return execution.into_world().unwrap();
        }
    }
    panic!("bounded retained resize did not return");
}

#[test]
fn retained_resize_sequence_matches_native_cuts_counts_and_live_ownership() {
    for publish in [false, true] {
        let mut world = raw_world();
        let construction = world.construction().clone();
        let spans = world.retained_spans().unwrap();
        for (requested, boundary_count) in [
            (64, 0),
            (48, 2880),
            (64, 0),
            (16, 960),
            (16, 0),
            (4, 240),
            (64, 0),
        ] {
            let old = world.active_size();
            let context = incoming(&world, requested, publish);
            let mut execution = resize::Execution::new(world, context).unwrap();
            let mut cuts = vec![];
            for _ in 0..8 {
                let reached = execution.step().unwrap().unwrap();
                cuts.push(reached);
                let restored: resize::Execution =
                    serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
                assert_eq!(execution, restored);
                restored.validate().unwrap();
                execution = restored;
                if reached == Cut::Returned {
                    break;
                }
            }
            let changed = old != requested as i32;
            assert_eq!(
                cuts,
                if changed {
                    vec![
                        Cut::Entry,
                        Cut::SizePublished,
                        Cut::BoundaryComplete,
                        Cut::GrassComplete,
                        Cut::CoordinatesComplete,
                        Cut::SizeRepublished,
                        Cut::BoundsComplete,
                        Cut::Returned,
                    ]
                } else {
                    vec![Cut::Entry, Cut::Returned]
                }
            );
            assert_eq!(execution.returned_al(), Some(1));
            assert_eq!(execution.boundary_writes().len(), boundary_count);
            assert_eq!(
                execution.coordinate_writes().len(),
                if changed { 24576 } else { 0 }
            );
            assert_eq!(
                execution.world_writes().len(),
                if changed {
                    if publish { 14 } else { 2 }
                } else {
                    0
                }
            );
            assert_eq!(
                execution.corner_calls().len(),
                if changed && publish { 4 } else { 0 }
            );
            assert_eq!(execution.tables(), execution.source().tables());
            assert_eq!(
                execution.arrays().context,
                execution.source().arrays().context
            );
            world = execution.into_world().unwrap();
            world.validate().unwrap();
            assert_eq!(world.active_size(), requested as i32);
            assert_eq!(world.construction(), &construction);
            assert_eq!(world.retained_spans().unwrap(), spans);
            let bounds: Vec<_> = world.owner().data[112..128]
                .chunks_exact(4)
                .map(|bytes| i32::from_le_bytes(bytes.try_into().unwrap()))
                .collect();
            assert_eq!(
                bounds,
                [
                    0,
                    if publish { requested as i32 - 1 } else { 63 },
                    0,
                    if publish { requested as i32 - 1 } else { 63 }
                ]
            );
        }
        assert_eq!(world.resizes().len(), 7);
    }
}

#[test]
fn resize_preserves_padding_roof_high_bytes_and_saved_flag_planes() {
    let world = raw_world();
    let before = world.clone();
    let world = resized(world, 48, true);
    for slot in [0x34, 0x38] {
        assert_eq!(matrix(&world, slot), matrix(&before, slot));
    }
    for slot in [0x58, 0x5c] {
        let Allocation::Known(earlier) = &matrix(&before, slot).allocation else {
            panic!()
        };
        let Allocation::Known(current) = &matrix(&world, slot).allocation else {
            panic!()
        };
        assert_eq!(
            earlier.data.iter().skip(1).step_by(2).collect::<Vec<_>>(),
            current.data.iter().skip(1).step_by(2).collect::<Vec<_>>()
        );
    }
    assert_eq!(&world.owner().data[128..], &before.owner().data[128..]);
    for level in 0..2 {
        assert_eq!(
            world.arrays().vectors[level].as_ref().unwrap().owner,
            before.arrays().vectors[level].as_ref().unwrap().owner
        );
    }
}

#[test]
fn resize_uses_actual_corner_layout_and_preserves_all_fourth_bytes() {
    let world = raw_world();
    let context = incoming(&world, 48, true);
    let initial = context.corners.clone().unwrap();
    let mut execution = resize::Execution::new(world, context).unwrap();
    while execution.step().unwrap() != Some(Cut::Returned) {}
    for (index, call) in execution.corner_calls().iter().enumerate() {
        assert_eq!(call.owner, initial[index].token);
        assert_eq!(call.returned_eax, call.owner);
        assert_eq!(call.argument, 0);
        assert_eq!(call.before, call.after);
        assert_eq!(call.before[3], initial[index].data[3]);
        assert_eq!(
            call.before[..3],
            [[0, 0, 0], [47, 0, 0], [47, 47, 0], [0, 47, 0]][index]
        );
    }
}

#[test]
fn resize_rejects_unqualified_profiles_before_any_effects() {
    let world = raw_world();
    for requested in [0, 3, 65, 128, u32::MAX] {
        let context = incoming(&world, requested, false);
        assert!(resize::Execution::new(world.clone(), context).is_err());
    }
    let context = incoming(&world, 48, true);
    let mut invalid = context.clone();
    invalid.current_world = world.owner().token as u32 + 4;
    assert!(resize::Execution::new(world.clone(), invalid).is_err());
    let mut invalid = context.clone();
    invalid.corners = None;
    assert!(resize::Execution::new(world.clone(), invalid).is_err());
    let mut invalid = context.clone();
    invalid.corners.as_mut().unwrap()[0].token += 4;
    assert!(resize::Execution::new(world.clone(), invalid).is_err());
    let mut invalid = context.clone();
    for corner in invalid.corners.as_mut().unwrap() {
        corner.token = world.owner().token + corner.token - 0x7a000000;
    }
    assert!(resize::Execution::new(world.clone(), invalid).is_err());
    let mut invalid = context;
    invalid.corners.as_mut().unwrap()[0].data.pop();
    assert!(resize::Execution::new(world, invalid).is_err());
}

#[test]
fn no_op_and_growth_do_not_replay_shrink_or_invent_corner_frames() {
    let world = resized(raw_world(), 48, true);
    let same = resized(world.clone(), 48, false);
    assert_eq!(same.arrays(), world.arrays());
    assert_eq!(same.owner(), world.owner());
    let grown = resized(same, 64, false);
    assert_eq!(grown.arrays(), world.arrays());
    assert_eq!(&grown.owner().data[112..128], &world.owner().data[112..128]);
    assert_eq!(grown.active_size(), 64);
}

#[test]
fn saved_arrays_borrow_resized_registry_without_replacing_resize_provenance() {
    let mut world = resized(raw_world(), 48, true);
    let resized_arrays = world.arrays().clone();
    let owner = world.owner().clone();
    let mut host = SourceHost::new(64);
    let mut execution = saved_world::Execution::new(63).unwrap();
    finish(
        &mut world,
        &mut execution,
        &mut Scratch {
            requested_converter: 0,
        },
        &mut host,
    );
    assert_eq!(world.active_size(), 48);
    assert_eq!(world.owner(), &owner);
    assert_eq!(world.resizes().len(), 1);
    assert_eq!(world.resizes()[0].arrays(), &resized_arrays);
    for slot in [0x2c, 0x30, 0x4c, 0x50, 0x58, 0x5c] {
        let identity = world.arrays().identity(slot).unwrap();
        assert_eq!(
            matrix(&world, slot),
            &resized_arrays.matrices.nodes[&identity].matrix
        );
    }
    assert_eq!((host.sources, host.releases, host.names), (1, 1, 0));
    assert_eq!(world.namings().len(), 1);
    world.validate().unwrap();
}

#[test]
fn altered_resize_effects_and_truncated_source_are_rejected_without_panics() {
    let world = raw_world();
    let context = incoming(&world, 48, true);
    let mut execution = resize::Execution::new(world, context).unwrap();
    while execution.step().unwrap() != Some(Cut::BoundaryComplete) {}
    for field in ["source_owner", "owner"] {
        let mut encoded = serde_json::to_value(&execution).unwrap();
        encoded["transition"][field]["data"] = serde_json::json!([]);
        let invalid: resize::Execution = serde_json::from_value(encoded).unwrap();
        assert!(invalid.validate().is_err());
    }
    let mut encoded = serde_json::to_value(&execution).unwrap();
    encoded["transition"]["boundary_writes"][0]["write"]["instruction"] = 0.into();
    let invalid: resize::Execution = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
    assert!(execution.into_world().is_err());
}

#[test]
fn repeated_resize_history_cannot_be_reordered_or_mutated() {
    let world = resized(resized(raw_world(), 48, true), 16, true);
    let mut encoded = serde_json::to_value(&world).unwrap();
    encoded["resizes"].as_array_mut().unwrap().swap(0, 1);
    let invalid: startup::World = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
    let mut encoded = serde_json::to_value(&world).unwrap();
    let identity = world.arrays().identity(0x2c).unwrap().to_string();
    encoded["arrays"]["matrices"]["nodes"][&identity]["matrix"]["allocation"]["Known"]["data"][0] =
        1.into();
    let invalid: startup::World = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
}

#[test]
fn malformed_required_lights_and_orphaned_roots_return_stops_instead_of_panicking() {
    let world = raw_world();
    let context = incoming(&world, 48, true);
    let mut execution = resize::Execution::new(world, context).unwrap();
    while execution.step().unwrap() != Some(Cut::Returned) {}
    let world = execution.clone().into_world().unwrap();
    for mutation in 0..3 {
        let mut encoded = serde_json::to_value(&execution).unwrap();
        let arrays = &mut encoded["transition"]["source_arrays"];
        match mutation {
            0 => arrays["vectors"][0] = serde_json::Value::Null,
            1 => arrays["vectors"][1]["backing"] = serde_json::Value::Null,
            _ => {
                arrays["matrices"]["nodes"] = serde_json::json!({});
                arrays["matrices"]["head"] = serde_json::Value::Null;
            }
        }
        let invalid: resize::Execution = serde_json::from_value(encoded.clone()).unwrap();
        assert!(invalid.validate().is_err());
        let state: resize::State =
            serde_json::from_value(serde_json::json!({"transition": encoded["transition"]}))
                .unwrap();
        assert!(state.validate().is_err());
        let mut retained = serde_json::to_value(&world).unwrap();
        retained["resizes"][0] = serde_json::to_value(state).unwrap();
        let invalid: startup::World = serde_json::from_value(retained).unwrap();
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn fourth_corner_must_alias_the_requested_size_argument_bytes() {
    let world = raw_world();
    let mut context = incoming(&world, 48, true);
    context.corners.as_mut().unwrap()[3].data[3] = 1;
    assert!(resize::Execution::new(world.clone(), context).is_err());
    let mut context = incoming(&world, 48, true);
    context.corners.as_mut().unwrap()[3].data[0] = 47;
    assert!(resize::Execution::new(world, context).is_err());
}

#[test]
fn current_world_global_cannot_alias_an_actual_resize_caller_frame() {
    let world = raw_world();
    let mut context = incoming(&world, 48, true);
    for corner in context.corners.as_mut().unwrap() {
        corner.token = 0x6e5528 + corner.token - 0x7a000000;
    }
    assert!(resize::Execution::new(world.clone(), context).is_err());
    let world = resized(world, 48, true);
    let mut encoded = serde_json::to_value(&world.resizes()[0]).unwrap();
    for corner in encoded["transition"]["incoming"]["corners"]
        .as_array_mut()
        .unwrap()
    {
        corner["token"] = (0x6e5528 + corner["token"].as_u64().unwrap() - 0x7a000000).into();
    }
    let invalid: resize::State = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
}
