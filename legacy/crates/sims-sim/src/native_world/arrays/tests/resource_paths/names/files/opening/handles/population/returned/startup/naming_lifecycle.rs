use super::*;
use crate::native_world::{
    resize as resizing,
    startup::{naming, reader},
};

fn seed(host: &mut SourceHost, world: &startup::World) {
    for node in world.arrays().matrices.nodes.values() {
        host.live_names
            .entry(node.name.token)
            .or_insert_with(|| node.name.data.clone());
    }
}

fn named(world: startup::World, host: &mut SourceHost) -> startup::World {
    seed(host, &world);
    let mut owner = naming::Owner::new(world).unwrap();
    for _ in 0..256 {
        if owner.step(host).unwrap() {
            return owner.into_world().unwrap();
        }
    }
    panic!("retained naming did not return");
}

fn resized(world: startup::World, requested: u32) -> startup::World {
    let mut execution = resizing::Execution::new(
        world,
        resizing::Incoming {
            requested,
            current_world: 0,
            corners: None,
        },
    )
    .unwrap();
    for _ in 0..8 {
        if execution.step().unwrap() == Some(resizing::Cut::Returned) {
            return execution.into_world().unwrap();
        }
    }
    panic!("retained resize did not return");
}

#[test]
fn naming_owner_retains_source_and_returns_only_a_quiescent_world() {
    let world = raw_world();
    let original = world.clone();
    let spans = world.retained_spans().unwrap();
    let mut host = SourceHost::new(64);
    seed(&mut host, &world);
    let mut owner = naming::Owner::new(world).unwrap().into_world().unwrap_err();
    assert_eq!(owner.source(), &original);
    for step in 0..256 {
        let complete = owner.step(&mut host).unwrap();
        if [0, 4, 30].contains(&step) {
            assert!(owner.retained_spans().unwrap().len() > spans.len());
            let restored: naming::Owner =
                serde_json::from_slice(&serde_json::to_vec(&owner).unwrap()).unwrap();
            assert_eq!(restored, owner);
            owner = restored;
        }
        if complete {
            break;
        }
    }
    let world = owner.into_world().unwrap();
    assert_eq!(world.owner(), original.owner());
    assert_eq!(world.arrays(), original.arrays());
    assert_eq!(world.construction(), original.construction());
    assert_eq!(world.namings().len(), 1);
    assert_eq!(world.retained_spans().unwrap(), spans);
    assert_eq!(
        (host.name_allocations, host.name_releases, host.names),
        (18, 18, 0)
    );
    let restored: startup::World =
        serde_json::from_slice(&serde_json::to_vec(&world).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(world, restored);
}

#[test]
fn naming_rejects_population_and_handle_alias_before_any_following_release() {
    let world = raw_world();
    for token in [
        world.construction().population().resources().token,
        world.construction().population().handle().owner().token,
    ] {
        let mut host = SourceHost::new(64);
        host.name_alias = Some(token);
        let mut owner = naming::Owner::new(world.clone()).unwrap();
        assert!(matches!(owner.step(&mut host), Err(Stop::Unsupported(_))));
        assert_eq!((host.name_allocations, host.name_releases), (1, 0));
        assert!(owner.step(&mut host).is_err());
        assert_eq!((host.name_allocations, host.name_releases), (1, 0));
        owner = owner.into_world().unwrap_err();
        assert_eq!(owner.source(), &world);
    }
}

#[test]
fn interleaved_naming_and_resize_returns_reject_reordering_and_source_mutation() {
    let original = raw_world();
    let mut host = SourceHost::new(64);
    let world = named(original.clone(), &mut host);
    let world = resized(world, 48);
    let world = named(world, &mut host);
    world.validate().unwrap();
    assert_eq!(
        (
            world.namings().len(),
            world.resizes().len(),
            world.active_size()
        ),
        (2, 1, 48)
    );
    assert_eq!(world.construction(), original.construction());
    let encoded = serde_json::to_value(&world).unwrap();
    let mut changed = encoded.clone();
    changed["history"].as_array_mut().unwrap().swap(0, 1);
    let invalid: startup::World = serde_json::from_value(changed).unwrap();
    assert!(invalid.validate().is_err());
    let mut changed = encoded.clone();
    changed["history"].as_array_mut().unwrap().remove(0);
    let invalid: startup::World = serde_json::from_value(changed).unwrap();
    assert!(invalid.validate().is_err());
    let mut changed = encoded;
    changed["namings"][1]["source_owner"]["data"][16] = 64.into();
    let invalid: startup::World = serde_json::from_value(changed).unwrap();
    assert!(invalid.validate().is_err());
}

#[test]
fn reader_keeps_waiting_cursor_and_pending_free_acknowledgment_together() {
    let world = raw_world();
    let original_owner = world.owner().clone();
    let mut host = SourceHost::new(64);
    seed(&mut host, &world);
    host.block_name_release = true;
    let mut read = reader::Owner::new(
        world,
        63,
        Scratch {
            requested_converter: 73,
        },
    )
    .unwrap();
    let mut stopped = false;
    for _ in 0..512 {
        match read.step(&mut host) {
            Err(Stop::Unavailable(_)) if host.pending_name_release.is_some() => {
                stopped = true;
                break;
            }
            Ok(None) => (),
            other => panic!("expected pending name release: {other:?}"),
        }
    }
    assert!(stopped && read.naming_in_progress());
    assert_eq!(
        (host.name_allocations, host.name_releases, host.names),
        (1, 1, 0)
    );
    let waiting = read.execution().clone();
    let encoded = serde_json::to_value(&read).unwrap();
    let mut changed = encoded.clone();
    changed["execution"]["copy_y"] = 99.into();
    assert!(serde_json::from_value::<reader::Owner>(changed).is_err());
    let mut changed = encoded.clone();
    changed["scratch"]["requested_converter"] = 99.into();
    assert!(serde_json::from_value::<reader::Owner>(changed).is_err());
    read = serde_json::from_value(encoded).unwrap();
    assert_eq!(read.execution(), &waiting);
    for _ in 0..256 {
        if read.step(&mut host).unwrap() == Some(0) {
            break;
        }
    }
    assert_eq!(read.status(), Some(0));
    assert_eq!(
        (host.name_allocations, host.name_releases, host.names),
        (18, 18, 0)
    );
    assert_eq!(read.step(&mut host).unwrap(), Some(0));
    assert_eq!((host.name_allocations, host.name_releases), (18, 18));
    let (world, execution, _) = read.into_returned().unwrap().into_parts();
    assert_eq!(world.owner(), &original_owner);
    assert_eq!(world.namings().len(), 1);
    assert_eq!(execution.status(), Some(0));
    assert_eq!(host.lookups.iter().filter(|&&id| id == 0).count(), 1);
}

#[test]
fn bare_world_cannot_acknowledge_names_through_a_generic_callback() {
    let mut world = raw_world();
    let mut host = SourceHost::new(64);
    host.missing = true;
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut scratch = Scratch {
        requested_converter: 0,
    };
    let mut stopped = false;
    for _ in 0..160 {
        match world.step_saved_arrays(&mut execution, &mut scratch, &mut host) {
            Err(Stop::Unavailable(_)) => {
                stopped = true;
                break;
            }
            Ok(None) => (),
            other => panic!("bare World must stop at its naming request: {other:?}"),
        }
    }
    assert!(stopped);
    assert_eq!(execution.status(), None);
    assert_eq!(host.names, 0);
    world.validate().unwrap();
}

#[test]
fn unsupported_reader_begin_returns_world_scratch_and_selected_version() {
    let world = raw_world();
    let scratch = Scratch {
        requested_converter: 17,
    };
    let error = reader::Owner::new(world.clone(), 52, scratch).unwrap_err();
    assert!(matches!(error.stop, Stop::Unsupported(_)));
    assert_eq!(error.world, world);
    assert_eq!(error.scratch, scratch);
    assert_eq!(error.version, 52);
}

fn corrupt_released_temporary(effects: &mut serde_json::Value, alias: u64) {
    let values = effects.as_array_mut().unwrap();
    let token = values[0]["Allocate"]["reply"]["Bytes"]["token"]
        .as_u64()
        .unwrap();
    values[0]["Allocate"]["reply"]["Bytes"]["token"] = alias.into();
    let release = values
        .iter_mut()
        .find(|effect| effect["Release"]["token"].as_u64() == Some(token))
        .unwrap();
    release["Release"]["token"] = alias.into();
}

#[test]
fn restored_released_name_temporaries_cannot_hide_resource_alias_history() {
    let world = raw_world();
    let alias = world.construction().population().resources().token;
    let mut host = SourceHost::new(64);
    seed(&mut host, &world);
    let mut owner = naming::Owner::new(world).unwrap();
    for _ in 0..7 {
        owner.step(&mut host).unwrap();
    }
    assert_eq!(host.name_releases, 1);
    let mut encoded = serde_json::to_value(&owner).unwrap();
    corrupt_released_temporary(&mut encoded["child"]["effects"], alias);
    assert!(serde_json::from_value::<naming::Owner>(encoded).is_err());
    for _ in 0..256 {
        if owner.step(&mut host).unwrap() {
            break;
        }
    }
    let world = owner.into_world().unwrap();
    let mut encoded = serde_json::to_value(&world).unwrap();
    corrupt_released_temporary(
        &mut encoded["namings"][0]["returned"]["execution"]["effects"],
        alias,
    );
    let invalid: startup::World = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().is_err());
}

#[test]
fn restored_reader_rejects_foreign_schedule_and_active_read_before_callbacks() {
    let world = raw_world();
    let mut read = reader::Owner::new(
        world,
        63,
        Scratch {
            requested_converter: 0,
        },
    )
    .unwrap();
    let encoded = serde_json::to_value(&read).unwrap();
    for path in ["id", "converter"] {
        let mut changed = encoded.clone();
        changed["execution"]["operations"][0]["Read"][path] = 4.into();
        assert!(serde_json::from_value::<reader::Owner>(changed).is_err());
    }
    let mut changed = encoded;
    changed["execution"]["version"] = 52.into();
    assert!(serde_json::from_value::<reader::Owner>(changed).is_err());
    let mut host = SourceHost::new(64);
    read.step(&mut host).unwrap();
    assert!(host.lookups.is_empty());
    let encoded = serde_json::to_value(&read).unwrap();
    for field in ["tag", "id", "converter"] {
        let mut changed = encoded.clone();
        changed["execution"]["read"][field] = 4.into();
        assert!(serde_json::from_value::<reader::Owner>(changed).is_err());
    }
    assert!(host.lookups.is_empty());
}
