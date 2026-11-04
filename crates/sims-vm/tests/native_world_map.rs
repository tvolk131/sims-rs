use serde_json::Value;
use sims_vm::{World, world_map::State};

#[cfg(feature = "private-fixtures")]
#[test]
fn retained_size_matches_original_dispatch_and_early_field_publication() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/world-map.json")).unwrap();
    assert_eq!(fixture["instructions_unchanged"], true);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 27);
    for case in cases {
        let old = case["old"].as_i64().unwrap() as i32;
        let requested = case["requested"].as_i64().unwrap() as i32;
        let mut state = State { active_size: old };
        let update = state.begin_resize(requested);
        assert_eq!(
            state.active_size,
            case["active_size"].as_i64().unwrap() as i32
        );
        assert_eq!(
            update.is_some(),
            !case["dispatch"].as_array().unwrap().is_empty()
        );
        if let Some(update) = update {
            assert_eq!((update.old, update.new), (old, requested));
            assert_eq!(case["publication"]["stored"], true);
        }
    }
}

#[test]
fn map_owner_survives_restore_without_borrowing_global_or_allocation_size() {
    let mut world = World {
        map_size: Some(State { active_size: 48 }),
        ..Default::default()
    };
    world.globals.insert(23, 64);
    let restored: World = serde_json::from_slice(&serde_json::to_vec(&world).unwrap()).unwrap();
    assert_eq!(restored, world);
    assert_eq!(restored.map_size.unwrap().checked_size(64).unwrap(), 48);
    assert!(restored.map_size.unwrap().checked_size(32).is_err());
    for size in [-1, 0, 3, 65] {
        assert!(State { active_size: size }.checked_size(64).is_err());
    }
    //Historical research states do not acquire invented resize history.
    let old = World::default();
    let bytes = serde_json::to_vec(&old).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.get("map_size").is_none());
    assert_eq!(
        serde_json::from_slice::<World>(&bytes).unwrap().map_size,
        None
    );
}
