use sims_sim::{lot_links::Links, lot_placement::State};
use sims_vm::{Definition, Module, Object, Program, World, autonomy::Placement};
use std::collections::BTreeMap;

fn fixture(c: &serde_json::Value) -> (State, World, Program) {
    let mut words = vec![0; 128];
    words[9] = if c["person"].as_bool().unwrap() { 2 } else { 4 };
    words[0x86 / 2] = c["size"].as_i64().unwrap() as i16;
    words[0x9a / 2] = c["mask"].as_u64().unwrap() as i16;
    let module = Module {
        source: "factory".into(),
        object_definition_words: BTreeMap::from([(1, words)]),
        ..Default::default()
    };
    let program = Program {
        modules: BTreeMap::from([("factory".into(), module)]),
        ..Default::default()
    };
    let object = Object {
        definition: Some(Definition {
            source: "factory".into(),
            object: 1,
            guid: 0,
            slots: 0,
        }),
        data: BTreeMap::from([(1, c["facing"].as_i64().unwrap() as i16)]),
        person: c["person"].as_bool().unwrap().then(|| {
            BTreeMap::from([
                (68, i16::from(c["zero_size"].as_bool().unwrap())),
                (73, c["extension"].as_u64().unwrap() as i16),
                (84, 0),
            ])
        }),
        ..Default::default()
    };
    let world = World {
        objects: BTreeMap::from([(1, object)]),
        positions: BTreeMap::from([(
            1,
            Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        )]),
        ..Default::default()
    };
    let state = State {
        links: Links {
            size: 4,
            heads: vec![vec![0; 16]; 2],
            next: BTreeMap::from([(1, 0)]),
        },
        bounds: BTreeMap::from([(9, [71, 72, 73, 74])]),
        flags: vec![vec![0x27; 16], vec![6; 16]],
        spatial_dirty: Some(true),
    };
    (state, world, program)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn actual_postconstructor_bounds_match_and_only_publish_the_target_rectangle() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/postconstructed-bounds.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 120);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let (mut state, world, program) = fixture(&row["input"]);
        let before_world = world.clone();
        let before = state.clone();
        state.postconstruct_bounds(&world, &program, 1).unwrap();
        assert_eq!(
            serde_json::json!(state.bounds[&1]),
            row["original"]["bounds"],
            "{row}"
        );
        assert_eq!(world, before_world);
        let cold: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(cold, state);
        state.bounds.remove(&1);
        assert_eq!(state, before);
    }
}

#[test]
fn postconstructor_requires_complete_inputs_and_never_repairs_a_moved_object() {
    let c = serde_json::json!({"person":true,"zero_size":false,"extension":0,"size":8,"mask":0,"facing":0});
    for fault in 0..8 {
        let (mut state, mut world, mut program) = fixture(&c);
        state.bounds.insert(1, [101, 102, 103, 104]);
        match fault {
            0 => {
                state.links.next.remove(&1);
            }
            1 => {
                world.objects.remove(&1);
            }
            2 => {
                world.positions.remove(&1);
            }
            3 => {
                world.positions.get_mut(&1).unwrap().x = 24;
            }
            4 => {
                world.objects.get_mut(&1).unwrap().definition = None;
            }
            5 => {
                world.objects.get_mut(&1).unwrap().data.insert(1, 8);
            }
            6 => {
                world
                    .objects
                    .get_mut(&1)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap()
                    .remove(&84);
            }
            7 => {
                program
                    .modules
                    .get_mut("factory")
                    .unwrap()
                    .object_definition_words
                    .get_mut(&1)
                    .unwrap()
                    .truncate(70);
            }
            _ => unreachable!(),
        }
        let before = state.clone();
        let before_world = world.clone();
        assert!(
            state.postconstruct_bounds(&world, &program, 1).is_err(),
            "fault {fault}"
        );
        assert_eq!(state, before);
        assert_eq!(world, before_world);
    }
}
