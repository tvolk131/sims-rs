use serde_json::{Value, json};
use sims_vm::{
    Object, ObjectId, World,
    presenter::{OrderView, RegistryOrder},
};

struct View {
    rotation: u32,
    size: i32,
    order: Vec<ObjectId>,
    tiles: Vec<[i8; 3]>,
}
impl OrderView for View {
    fn rotation(&self) -> u32 {
        self.rotation
    }
    fn size(&self) -> i32 {
        self.size
    }
    fn registered(&self) -> &[ObjectId] {
        &self.order
    }
    fn cached_tile(&self, id: ObjectId) -> Result<[i8; 3], String> {
        self.tiles
            .get(id as usize - 1)
            .copied()
            .ok_or("test tile absent".into())
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn registry_owner_matches_all_native_getter_cases_and_cold_state() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/presenter-order.json")).unwrap();
    let mut cases = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        if c.get("heap").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let view = View {
            rotation: c["rotation"].as_u64().unwrap() as u32,
            size: c["size"].as_i64().unwrap() as i32,
            order: serde_json::from_value(c["order"].clone()).unwrap(),
            tiles: serde_json::from_value(c["tiles"].clone()).unwrap(),
        };
        let mut cache = RegistryOrder {
            dirty: c["dirty"].as_u64().unwrap() != 0,
            rotation: Some(c["cached_rotation"].as_u64().unwrap() as u32),
            objects: serde_json::from_value(c["cached"].clone()).unwrap(),
        };
        let mut cold: RegistryOrder =
            serde_json::from_slice(&serde_json::to_vec(&cache).unwrap()).unwrap();
        assert_eq!(cache.get(&view), cold.get(&view));
        assert_eq!(cache, cold);
        assert_eq!(
            json!({"order":cache.objects,"rotation":cache.rotation.unwrap(),"dirty":cache.dirty}),
            row["original"]
        );
        cases += 1;
    }
    assert_eq!(cases, 240);
}

#[test]
fn constructor_rotation_is_unwritten_and_writer_preserves_stale_vector() {
    let mut cache = RegistryOrder::constructed();
    assert!(cache.dirty);
    assert_eq!(cache.rotation, None);
    let mut view = View {
        rotation: 0,
        size: 48,
        order: vec![1, 2],
        tiles: vec![[2, 2, 1], [-1, -1, 1]],
    };
    assert_eq!(cache.get(&view).unwrap(), [1]);
    view.tiles[1] = [3, 3, 1];
    //Clean getter has the original stale-read behavior until actual invalidation.
    assert_eq!(cache.get(&view).unwrap(), [1]);
    cache.invalidate();
    assert_eq!(cache.objects, [1]);
    assert_eq!(cache.rotation, Some(0));
    let mut cold: RegistryOrder =
        serde_json::from_slice(&serde_json::to_vec(&cache).unwrap()).unwrap();
    assert_eq!(cold.get(&view).unwrap(), [1, 2]);
    assert_eq!(cache.get(&view).unwrap(), [1, 2]);
    assert_eq!(cache, cold);
}

#[test]
fn clean_getter_does_not_read_registry_tiles_or_size() {
    struct Trap;
    impl OrderView for Trap {
        fn rotation(&self) -> u32 {
            2
        }
        fn size(&self) -> i32 {
            panic!("clean getter read size")
        }
        fn registered(&self) -> &[ObjectId] {
            panic!("clean getter enumerated")
        }
        fn cached_tile(&self, _: ObjectId) -> Result<[i8; 3], String> {
            panic!("clean getter read tiles")
        }
    }
    let mut cache = RegistryOrder {
        dirty: false,
        rotation: Some(2),
        objects: vec![42],
    };
    assert_eq!(cache.get(&Trap).unwrap(), [42]);
}

#[test]
fn world_invalidation_does_not_initialize_unknown_historical_ownership() {
    let mut world = World::default();
    let before = serde_json::to_value(&world).unwrap();
    world.invalidate_registry_order();
    assert_eq!(serde_json::to_value(&world).unwrap(), before);
    world.registry_order = Some(RegistryOrder {
        dirty: false,
        rotation: Some(0),
        objects: vec![1],
    });
    assert!(world.validate_registry_order().is_err());
    world.objects.insert(1, Object::default());
    world.validate_registry_order().unwrap();
    world.invalidate_registry_order();
    world.objects.remove(&1);
    //Native removal leaves the old vector stale, rather than erasing its IDs.
    world.validate_registry_order().unwrap();
    assert_eq!(world.registry_order.as_ref().unwrap().objects, [1]);
    let cold: World = serde_json::from_slice(&serde_json::to_vec(&world).unwrap()).unwrap();
    assert_eq!(cold, world);
}

#[test]
fn failed_rebuild_retains_its_partial_native_publication() {
    let view = View {
        rotation: 0,
        size: 48,
        order: vec![1, 2],
        tiles: vec![[2, 2, 1]],
    };
    let mut cache = RegistryOrder::constructed();
    assert_eq!(cache.get(&view), Err("test tile absent".into()));
    assert!(!cache.dirty);
    assert_eq!(cache.rotation, Some(0));
    assert_eq!(cache.objects, [1]);
}
