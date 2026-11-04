use serde_json::{Value, json};
use sims_vm::{
    Object, World, person,
    person_loading::{RoomEffect, RoomMembership, score_table},
};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "private-fixtures")]
#[test]
fn postload_room_and_score_decisions_match_original() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/person-postload.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 99);
    for case in cases {
        let c = &case["input"];
        let n = |key: &str| c[key].as_i64().unwrap() as i16;
        let mut world = World {
            objects: BTreeMap::from([
                (
                    1,
                    Object {
                        data: BTreeMap::from([(29, n("room"))]),
                        person: Some(BTreeMap::from([(65, n("species")), (68, 0), (84, 0)])),
                        ..Default::default()
                    },
                ),
                (
                    2,
                    Object {
                        person: Some(BTreeMap::from([
                            (68, i16::from(!c["other"].as_bool().unwrap())),
                            (84, 0),
                        ])),
                        ..Default::default()
                    },
                ),
            ]),
            person: Some(person::WorldState {
                person_order: vec![2, 1],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut membership =
            RoomMembership(BTreeMap::from([(1, n("old_room")), (2, n("old_room"))]));
        let effects = membership
            .update(
                &world,
                1,
                &BTreeSet::from([1, 2]),
                c["selected"].as_bool().unwrap(),
            )
            .unwrap();
        let mut events = vec![];
        for effect in effects {
            events.push(match effect {
                RoomEffect::Occupied { room, occupied } => {
                    json!(["occupied", room, i32::from(occupied)])
                }
                RoomEffect::SelectedRoomChanged => json!(["signal", 247, 0]),
            });
        }
        let (global, table) = score_table(n("age"), n("custom") as u16);
        let original = case["original"]["events"].as_array().unwrap();
        assert_eq!(
            json!(["scores", if global { "global" } else { "private" }, table]),
            *original.iter().find(|e| e[0] == "scores").unwrap(),
            "{c}"
        );
        let expected: Vec<_> = original
            .iter()
            .filter(|e| e[0] == "occupied" || e[0] == "signal")
            .cloned()
            .collect();
        assert_eq!(events, expected, "{c}");
        assert_eq!(json!(membership.0[&1]), case["original"]["room"], "{c}");
        // A later visit to the same room emits nothing; both the cache and the
        // neighboring person's membership survive serialization independently.
        membership = serde_json::from_value(json!(membership)).unwrap();
        world
            .objects
            .get_mut(&1)
            .unwrap()
            .data
            .insert(29, membership.0[&1]);
        assert!(
            membership
                .update(&world, 1, &BTreeSet::from([1, 2]), true)
                .unwrap()
                .is_empty()
        );
    }
}
