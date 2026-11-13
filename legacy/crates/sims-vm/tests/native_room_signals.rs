use serde_json::{Value, json};
use sims_vm::{
    Object, Program, Snapshot, Vm,
    refresh::{RefreshState, RoomState},
};
use std::collections::BTreeMap;

fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (index, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let n = |key: &str| c[key].as_i64().unwrap() as i32;
        let room = n("registered_room") as u16;
        let mut vm = Vm::new(Default::default(), 1);
        for id in 1..=2 {
            if id == 2 && c["second_exists"] == false {
                continue;
            }
            vm.world.objects.insert(
                id,
                Object {
                    data: BTreeMap::from([(29, n("object_room") as i16)]),
                    ..Default::default()
                },
            );
        }
        vm.world.refresh = Some(RefreshState {
            rooms: BTreeMap::from([(
                room,
                RoomState {
                    category: n("category") as u8,
                    dirty: c["dirty"] == true,
                },
            )]),
            dirty_categories: n("categories") as u32,
            ..Default::default()
        });
        let result = if c["primitive"] == true {
            vm.refresh_object(1, n("value") as i16, 1, 1)
        } else {
            vm.refresh_room_event(n("event") as u16, n("value"))
        };
        let error = result.as_ref().err().map(|e| {
            let value = e
                .strip_prefix("native error 0x")
                .expect(e)
                .split(':')
                .next()
                .unwrap();
            u32::from_str_radix(value, 16).unwrap()
        });
        let state = vm.world.refresh.as_ref().unwrap();
        let original = &row["original"];
        assert_eq!(
            json!({"error":error,"returned":result.is_ok().then_some(1),"dirty":state.rooms[&room].dirty,
            "categories":state.dirty_categories,"redraw":state.redraw_rooms}),
            json!({"error":original["error"],"returned":original["returned"],"dirty":original["dirty"],
            "categories":original["categories"],"redraw":original["redraw"]}),
            "room signal {index}: {c}"
        );
        let events = original["events"].as_array().unwrap();
        let handlers = original["handlers"].as_array().unwrap();
        assert_eq!(
            handlers.len(),
            events.len() * 8,
            "all eight native subscribers must run"
        );
        for id in 0..8 {
            assert_eq!(handlers.iter().filter(|h| h[0] == id).count(), events.len());
        }
        let p = Program::default();
        let snapshot = Snapshot::capture(&p, &vm);
        let restored: Snapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert_eq!(restored.restore(&p).unwrap(), vm);
        let mut bad = snapshot;
        bad.state
            .world
            .refresh
            .as_mut()
            .unwrap()
            .redraw_rooms
            .insert(60000);
        assert!(bad.restore(&p).is_err());
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_room_notifications_and_refresh_mode_one() {
    compare(serde_json::from_str(include_str!("fixtures/room-signals.json")).unwrap());
}
#[test]
#[ignore = "requires private original Commander/room oracle"]
fn native_room_notifications_full_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/room-signals.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}
