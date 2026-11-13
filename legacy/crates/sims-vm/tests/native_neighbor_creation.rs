use serde_json::{Value, json};
use sims_vm::{Program, Snapshot, Vm, World, person, person_factory::NeighborAttachment};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn allocation_and_both_attachment_paths_match_original() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/neighbor-creation.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 126);
    for row in rows {
        let c = &row["input"];
        let n = &row["original"];
        let slots: Vec<_> = c["slots"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, present)| (present == 1).then_some(i as i16 + 1))
            .collect();
        let mut neighborhood = person::WorldState {
            neighbor_slots: Some(slots.clone()),
            ..Default::default()
        };
        for id in slots.iter().flatten().copied() {
            neighborhood.neighbors.insert(
                id,
                person::Neighbor {
                    guid: if c["match"].as_i64() == Some(i64::from(id - 1)) {
                        0x44a86a6c
                    } else {
                        id as u32
                    },
                    person_revision: c["revision"].as_i64().unwrap_or(0) as i32,
                    person: (0..256).map(|i| (i, i + 1000)).collect(),
                    ..Default::default()
                },
            );
        }
        let initial_count = neighborhood.neighbors.len();
        if c["kind"] == "create" {
            let returned = neighborhood
                .create_neighbor(
                    (!c["null"].as_bool().unwrap()).then_some(0x44a86a6c),
                    c["requested"].as_i64().unwrap() as i16,
                )
                .unwrap();
            assert_eq!(json!(returned), n["returned"], "{c}");
        } else {
            let mut person: BTreeMap<_, _> = (0..256).map(|i| (i, i - 256)).collect();
            let mode = if c["kind"] == "persistent" {
                NeighborAttachment::Persistent
            } else {
                NeighborAttachment::Traits
            };
            neighborhood
                .attach_person_neighbor(&mut person, 0x44a86a6c, mode)
                .unwrap();
            assert_eq!(
                json!(person.values().copied().collect::<Vec<_>>()),
                n["person"],
                "{c}"
            );
        }
        assert_eq!(json!(neighborhood.neighbor_slots), n["slots"], "{c}");
        assert_eq!(
            json!(neighborhood.neighbors.len() - initial_count),
            n["allocated"],
            "{c}"
        );
        if n["allocated"] == 1 {
            let new = neighborhood
                .neighbors
                .values()
                .find(|n| n.guid == 0x44a86a6c)
                .unwrap();
            assert_eq!(json!(new.guid), n["new"]["guid"]);
            assert_eq!(json!(new.person_revision), n["new"]["revision"]);
            assert_eq!(
                json!(new.person.values().copied().collect::<Vec<_>>()),
                n["new"]["person"]
            );
            assert!(new.relationships.is_empty() && new.inventory.is_empty() && !new.dirty);
        }
        let vm = Vm::new(
            World {
                person: Some(neighborhood),
                ..Default::default()
            },
            7,
        );
        let program = Program::default();
        assert_eq!(
            Snapshot::capture(&program, &vm).restore(&program).unwrap(),
            vm
        );
    }
}

#[test]
fn snapshot_rejects_lost_or_duplicate_neighbor_slots() {
    let mut neighborhood = person::WorldState {
        neighbor_slots: Some(vec![None, None, None]),
        ..Default::default()
    };
    assert_eq!(neighborhood.create_neighbor(Some(7), -1).unwrap(), Some(3));
    let vm = Vm::new(
        World {
            person: Some(neighborhood),
            ..Default::default()
        },
        7,
    );
    let program = Program::default();
    for slots in [vec![None], vec![Some(3), Some(3)], vec![Some(2)]] {
        let mut saved = Snapshot::capture(&program, &vm);
        saved.state.world.person.as_mut().unwrap().neighbor_slots = Some(slots);
        assert!(saved.restore(&program).is_err());
    }
}
