use super::*;
use crate::native_world::arrays;
use crate::native_world::geometry::{Cut, Initialization};

fn payload(arrays: &arrays::State, slot: u8) -> &Vec<u8> {
    let identity = arrays.identity(slot).unwrap();
    let Allocation::Known(backing) = &arrays.matrices.nodes[&identity].matrix.allocation else {
        panic!()
    };
    &backing.data
}

#[test]
fn isolated_geometry_fills_full_backing_after_active_shrink_without_publishing_world() {
    let mut world = raw_world();
    let incoming = crate::native_world::resize::Incoming {
        requested: 4,
        current_world: 0,
        corners: None,
    };
    let mut resize = crate::native_world::resize::Execution::new(world, incoming).unwrap();
    while resize.step().unwrap() != Some(crate::native_world::resize::Cut::Returned) {}
    world = resize.into_world().unwrap();
    for floor in [0, 1] {
        let mut initialization = Initialization::isolated(world.clone(), floor).unwrap();
        assert_eq!(initialization.cut(), Some(Cut::Entry));
        assert_eq!(initialization.source().active_size(), 4);
        assert_eq!(initialization.floor(), floor);
        let mut writes = vec![];
        while let Some(write) = initialization.step().unwrap() {
            writes.push(write);
            if initialization.stores() == 2048 {
                assert_eq!(initialization.cut(), Some(Cut::PrimaryFilled));
            }
        }
        initialization.validate().unwrap();
        assert_eq!(initialization.cut(), Some(Cut::ScanEntry));
        assert_eq!(initialization.stores(), 6144);
        assert_eq!(initialization.source(), &world);
        assert_eq!(
            payload(initialization.arrays(), 0x2c + floor * 4),
            &vec![0xff; 8192]
        );
        assert_eq!(writes[2048].offset, 0);
        assert_eq!(writes[2049].offset, 64);
        assert_eq!(writes[2112].offset, 1);
        let mut changed_world = serde_json::to_value(&world).unwrap();
        changed_world["arrays"] = serde_json::to_value(initialization.arrays()).unwrap();
        let changed_world: startup::World = serde_json::from_value(changed_world).unwrap();
        assert!(changed_world.validate().is_err());
        for slot in [0x2c, 0x30, 0x34, 0x38, 0x4c, 0x50, 0x58, 0x5c] {
            if ![0x2c + floor * 4, 0x34 + floor * 4].contains(&slot) {
                assert_eq!(
                    payload(initialization.arrays(), slot),
                    payload(world.arrays(), slot)
                );
            }
        }
    }
    assert!(Initialization::isolated(world, 2).is_err());
}

#[test]
fn geometry_retains_exact_store_cursor_across_partial_fill_and_mask() {
    let world = raw_world();
    let mut initialization = Initialization::isolated(world, 1).unwrap();
    let mut events = vec![];
    for cut in [
        0, 1, 2, 2047, 2048, 2049, 2111, 2112, 2113, 4096, 6143, 6144,
    ] {
        while initialization.stores() < cut {
            events.push(initialization.step().unwrap().unwrap());
        }
        let restored: Initialization =
            serde_json::from_slice(&serde_json::to_vec(&initialization).unwrap()).unwrap();
        assert_eq!(restored, initialization);
        restored.validate().unwrap();
        initialization = restored;
    }
    assert_eq!(events.len(), 6144);
    assert!(initialization.step().unwrap().is_none());
    let mut uninterrupted = Initialization::isolated(initialization.source().clone(), 1).unwrap();
    for event in events {
        assert_eq!(uninterrupted.step().unwrap(), Some(event));
    }
    assert_eq!(uninterrupted, initialization);
}

#[test]
fn malformed_geometry_restore_is_rejected_before_mutation() {
    let mut initialization = Initialization::isolated(raw_world(), 0).unwrap();
    initialization.step().unwrap();
    let encoded = serde_json::to_value(&initialization).unwrap();
    let identity = initialization.arrays().roots[&0x2c].token.to_string();
    for fault in 0..8 {
        let mut changed = encoded.clone();
        match fault {
            0 => changed["stores"] = serde_json::json!(6145),
            1 => changed["floor"] = serde_json::json!(2),
            2 => changed["arrays"]["vectors"][0] = serde_json::Value::Null,
            3 => {
                changed["arrays"]["matrices"]["nodes"][&identity]["matrix"]["allocation"] =
                    serde_json::json!("Null")
            }
            4 => {
                changed["arrays"]["matrices"]["nodes"][&identity]["matrix"]["allocation"]["Known"]
                    ["data"][0] = serde_json::json!(0)
            }
            5 => changed["arrays"]["roots"]
                .as_object_mut()
                .unwrap()
                .remove("44")
                .map(|_| ())
                .unwrap(),
            6 => changed["stores"] = serde_json::json!(0),
            7 => changed["invented_room_manager"] = serde_json::json!(true),
            _ => unreachable!(),
        }
        let restored = serde_json::from_value::<Initialization>(changed);
        assert!(restored.is_err(), "fault {fault}");
    }
    assert_eq!(initialization.stores(), 1);
    initialization.validate().unwrap();
}
