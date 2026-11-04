use serde::Deserialize;
use sims_vm::person::{Neighbor, WorldState};

#[derive(Deserialize)]
struct InputNeighbor {
    guid: u32,
    family: i16,
}
#[derive(Deserialize)]
struct Family {
    id: i16,
    #[serde(default)]
    number: i32,
    members: Vec<u32>,
}
#[derive(Deserialize)]
struct Input {
    neighbors: Vec<Option<InputNeighbor>>,
    families: Vec<Family>,
}
#[derive(Deserialize)]
struct Row<T> {
    input: Input,
    original: T,
}
#[derive(Deserialize)]
struct Sorted {
    order: Vec<usize>,
    family_words: Vec<Option<i16>>,
    #[serde(default)]
    members: Vec<Vec<u32>>,
}
#[derive(Deserialize)]
struct Report {
    capture_sha256: String,
    cases: Vec<Row<Vec<Option<i16>>>>,
    sorted_cases: Vec<Row<Sorted>>,
    reload_cases: Vec<Row<Sorted>>,
}

fn world(input: &Input) -> WorldState {
    // Reverse numeric IDs so the first GUID lookup must follow the native
    // slot vector rather than the BTreeMap's key order.
    let slots: Vec<_> = input
        .neighbors
        .iter()
        .enumerate()
        .map(|(i, n)| n.as_ref().map(|_| (input.neighbors.len() - i) as i16))
        .collect();
    let mut world = WorldState {
        neighbor_slots: Some(slots.clone()),
        ..Default::default()
    };
    for (id, n) in slots.iter().zip(&input.neighbors) {
        if let (Some(id), Some(n)) = (id, n) {
            world.neighbors.insert(
                *id,
                Neighbor {
                    guid: n.guid,
                    dirty: id % 2 == 0,
                    person: [(60, -129), (61, n.family), (62, 317)].into(),
                    ..Default::default()
                },
            );
        }
    }
    world
}

fn run(input: &Input, order: &[usize]) -> Vec<Option<i16>> {
    let mut world = world(input);
    let slots = world.neighbor_slots.as_ref().unwrap().clone();
    let before = world.clone();
    world
        .rebuild_family_links(order.iter().map(|&i| {
            let f = &input.families[i];
            (f.id, f.members.as_slice())
        }))
        .unwrap();
    let words = slots
        .iter()
        .map(|id| id.map(|id| world.neighbors[&id].person[&61]))
        .collect();
    // Raw field stores must not dirty neighbors or change other saved data.
    for (&id, n) in &mut world.neighbors {
        n.person.insert(61, before.neighbors[&id].person[&61]);
    }
    assert_eq!(world, before);
    words
}

#[cfg(feature = "private-fixtures")]
#[test]
fn family_sort_and_rebuilding_match_original_reload_routines() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/neighborhood-family-links.json")).unwrap();
    assert_eq!(
        report.capture_sha256,
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report.cases.len(), 129);
    assert_eq!(report.sorted_cases.len(), 60);
    assert_eq!(report.reload_cases.len(), 61);
    for row in report.cases {
        let order: Vec<_> = (0..row.input.families.len()).collect();
        assert_eq!(run(&row.input, &order), row.original);
    }
    for row in report.sorted_cases {
        let keys: Vec<_> = row
            .input
            .families
            .iter()
            .map(|f| (f.id, f.number))
            .collect();
        let order = sims_vm::neighborhood::family_order(&keys).unwrap();
        assert_eq!(order, row.original.order, "{keys:?}");
        assert_eq!(run(&row.input, &order), row.original.family_words);
    }
    for row in report.reload_cases {
        let world = world(&row.input);
        let members: Vec<Vec<_>> = row
            .input
            .families
            .iter()
            .map(|f| {
                world
                    .resolve_family_members(&f.members)
                    .unwrap()
                    .iter()
                    .map(|id| world.neighbors[id].guid)
                    .collect()
            })
            .collect();
        assert_eq!(members, row.original.members);
        let keys: Vec<_> = row
            .input
            .families
            .iter()
            .map(|f| (f.id, f.number))
            .collect();
        let order = sims_vm::neighborhood::family_order(&keys).unwrap();
        assert_eq!(order, row.original.order);
        assert_eq!(run(&row.input, &order), row.original.family_words);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn family_preparation_and_later_registration_match_whole_reload_body() {
    use serde_json::{Value, json};
    let report: Value =
        serde_json::from_str(include_str!("fixtures/neighborhood-reload-body.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 48);
    // The later rows exercise native resource failures. They are retained as
    // evidence for a future reload owner, not claimed as Rust error-path coverage.
    for row in &report["cases"].as_array().unwrap()[..24] {
        let c = &row["input"];
        let expected = &row["original"];
        let input: Input = serde_json::from_value(c.clone()).unwrap();
        let mut world = WorldState {
            neighbor_slots: Some(
                input
                    .neighbors
                    .iter()
                    .enumerate()
                    .map(|(i, n)| n.as_ref().map(|_| i as i16 + 1))
                    .collect(),
            ),
            ..Default::default()
        };
        for (i, n) in input.neighbors.iter().enumerate() {
            if let Some(n) = n {
                world.neighbors.insert(
                    i as i16 + 1,
                    Neighbor {
                        guid: n.guid,
                        person: [(61, n.family)].into(),
                        ..Default::default()
                    },
                );
            }
        }
        let families: Vec<_> = input
            .families
            .iter()
            .map(|f| sims_vm::neighborhood::Family {
                id: f.id,
                number: f.number,
                house: 0,
                flags: 0,
                friends: 0, // explicit constructor-equivalent authored input
                members: f.members.clone(),
            })
            .collect();
        let cleaned = world.prepare_family_reload(&families).unwrap();
        let actual_families: Vec<_> = cleaned
            .iter()
            .map(|f| {
                json!({
                    "id": f.id, "number": f.number, "house": f.house, "members": f.members,
                })
            })
            .collect();
        assert_eq!(json!(actual_families), expected["families"], "{c}");
        // Native catalog filtering and allocation follow cleanup. Synthetic
        // resource identities here are unambiguous GUIDs; no filesystem order
        // or duplicate-GUID aliasing is inferred from these cases.
        for r in c["resources"].as_array().into_iter().flatten() {
            if !matches!(
                r["kind"].as_i64().unwrap(),
                2 | 0x200 | 0x400 | 0x800 | 0x1000
            ) {
                continue;
            }
            let guid = r["guid"].as_u64().unwrap() as u32;
            if !world.neighbors.values().any(|n| n.guid == guid) {
                world.create_neighbor(Some(guid), -1).unwrap();
            }
        }
        world
            .rebuild_family_links(cleaned.iter().map(|f| (f.id, f.members.as_slice())))
            .unwrap();
        let actual_neighbors: Vec<_> = world
            .neighbor_slots
            .as_ref()
            .unwrap()
            .iter()
            .map(|id| {
                id.map(|id| {
                    json!({"id":id,"guid":world.neighbors[&id].guid,
                "family":world.neighbors[&id].person[&61]})
                })
            })
            .collect();
        assert_eq!(json!(actual_neighbors), expected["neighbors"], "{c}");
        assert_eq!(json!(world.family_order), expected["family_order"]);
        if let Some(lines) = c["zoning"].as_array() {
            let lines: Vec<_> = lines
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect();
            let rows = sims_vm::zoning::parse(
                &lines,
                if c["magic_installed"].as_bool() == Some(true) {
                    0x80
                } else {
                    0
                },
            )
            .unwrap();
            assert_eq!(json!(rows), expected["zoning"]);
        }
        let program = sims_vm::Program::default();
        let vm = sims_vm::Vm::new(
            sims_vm::World {
                person: Some(world),
                ..Default::default()
            },
            7,
        );
        assert_eq!(
            sims_vm::Snapshot::capture(&program, &vm)
                .restore(&program)
                .unwrap(),
            vm
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn snapshot_preserves_native_repeated_and_overlapping_membership() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/neighborhood-family-links.json")).unwrap();
    let row = &report.reload_cases[0];
    let mut people = world(&row.input);
    people.family_order = Some(
        row.original
            .order
            .iter()
            .map(|&i| row.input.families[i].id)
            .collect(),
    );
    for family in &row.input.families {
        people.families.insert(
            family.id,
            people.resolve_family_members(&family.members).unwrap(),
        );
    }
    people
        .rebuild_family_links(row.original.order.iter().map(|&i| {
            let f = &row.input.families[i];
            (f.id, f.members.as_slice())
        }))
        .unwrap();
    assert_eq!(
        people.families[&2].len(),
        2,
        "native cleanup retains valid duplicates"
    );
    assert_eq!(people.families[&2][0], people.families[&2][1]);
    let member = people.families[&2][0];
    assert_eq!(
        people.neighbors[&member].person[&61], 1,
        "later family write must survive while the earlier family's list remains"
    );
    let vm = sims_vm::Vm::new(
        sims_vm::World {
            person: Some(people),
            ..Default::default()
        },
        7,
    );
    let program = sims_vm::Program::default();
    let snapshot = sims_vm::Snapshot::capture(&program, &vm);
    let serialized = serde_json::to_vec(&snapshot).unwrap();
    assert_eq!(
        serde_json::from_slice::<sims_vm::Snapshot>(&serialized)
            .unwrap()
            .restore(&program)
            .unwrap(),
        vm
    );
    let mut missing = snapshot.clone();
    missing
        .state
        .world
        .person
        .as_mut()
        .unwrap()
        .families
        .get_mut(&2)
        .unwrap()
        .push(123);
    assert!(missing.restore(&program).is_err());
    for order in [None, Some(vec![1]), Some(vec![1, 1]), Some(vec![1, 99])] {
        let mut lost = snapshot.clone();
        lost.state.world.person.as_mut().unwrap().family_order = order;
        assert!(lost.restore(&program).is_err());
    }
    let mut unrelated = snapshot;
    unrelated
        .state
        .world
        .person
        .as_mut()
        .unwrap()
        .neighbors
        .get_mut(&member)
        .unwrap()
        .person
        .insert(61, 99);
    assert!(unrelated.restore(&program).is_err());
}

#[test]
fn missing_or_invalid_slot_context_preserves_saved_words() {
    for slots in [None, Some(vec![]), Some(vec![Some(1), Some(1)])] {
        let mut world = WorldState {
            neighbor_slots: slots,
            neighbors: [(
                1,
                Neighbor {
                    guid: 7,
                    person: [(61, 99)].into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let before = world.clone();
        assert!(world.rebuild_family_links(std::iter::empty()).is_err());
        assert_eq!(world, before);
    }
    assert!(sims_vm::neighborhood::family_order(&[(1, 2), (1, 3)]).is_err());
}
