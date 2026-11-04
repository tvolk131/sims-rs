use serde_json::{Value, json};
use sims_vm::{
    action,
    autonomy::Enumeration,
    program::{AutonomyAdvertisement, MenuEntry},
    *,
};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_person_entries_and_active_action_join_remapping() {
    let report: Value = serde_json::from_str(include_str!("fixtures/autonomy-join.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let redirect = c["redirect"].as_i64().unwrap();
        let mut p = Program::default();
        let mut m = Module::default();
        let ordinary = vec![
            MenuEntry {
                id: 10,
                flags: 0,
                ..Default::default()
            },
            MenuEntry {
                id: 11,
                flags: 128,
                ..Default::default()
            },
            MenuEntry {
                id: 12,
                flags: 64,
                ..Default::default()
            },
        ];
        m.object_interactions.insert(2, ordinary.clone());
        m.object_interactions.insert(
            1,
            vec![
                MenuEntry {
                    id: 33,
                    flags: c["active_flags"].as_u64().unwrap() as u32,
                    autonomy: AutonomyAdvertisement {
                        joining_index: c["join_id"].as_i64().unwrap() as i32,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                MenuEntry {
                    id: 34,
                    flags: 256,
                    action: 4097,
                    ..Default::default()
                },
            ],
        );
        let mut world = World::default();
        for id in 1..=2 {
            let mut words = vec![0; 70];
            words[10] = i16::from(redirect != 0);
            words[8] = if id == 1 {
                1
            } else if redirect == 2 {
                -1
            } else {
                -2
            };
            m.object_definition_words.insert(id, words);
            world.objects.insert(
                id as i16,
                Object {
                    definition: Some(Definition {
                        source: "test".into(),
                        object: id,
                        guid: 0,
                        slots: 0,
                    }),
                    multitile: (redirect != 0).then_some(multitile::Part {
                        head: (id == 2).then_some(1),
                        next: (id == 1).then_some(2),
                        origin: [0; 3],
                        offset: [0; 3],
                    }),
                    ..Default::default()
                },
            );
        }
        p.modules.insert("test".into(), m);
        world.actions = Some(action::WorldState {
            actors: BTreeMap::from([(
                2,
                action::Actor {
                    active: c["active"].as_bool().unwrap().then_some(action::Action {
                        target: 1,
                        menu_id: c["active_menu"].as_u64().unwrap() as u32,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        });
        let vm = Vm::new(world, 1);
        let policy = Enumeration {
            age: c["age"].as_i64().unwrap() as i16,
            species: c["species"].as_i64().unwrap() as i16,
            visitor: c["visitor"].as_i64().unwrap() as i16,
            level: c["level"].as_i64().unwrap() as i16,
            live_family: true,
            pet_permission: 0,
            target_kind: 2,
            target_room: 1,
        };
        let eligible: Vec<_> = ordinary
            .iter()
            .rev()
            .filter(|e| policy.permits(e.flags, 50).unwrap())
            .map(|e| e.id)
            .collect();
        assert_eq!(json!(eligible), row["original"]["eligible"], "{c}");
        let native = &row["original"]["join"];
        let expected = (!native.is_null() && native["target"] != 0).then(|| {
            (
                native["target"].as_i64().unwrap() as i16,
                native["menu"].as_u64().unwrap() as u32,
            )
        });
        assert_eq!(vm.autonomy_join_target(&p, 2).unwrap(), expected, "{c}");
        assert!(row["original"]["error"].is_null());
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_five_word_eligibility_buffer() {
    let report: Value = serde_json::from_str(include_str!("fixtures/autonomy-join.json")).unwrap();
    for row in report["checks"].as_array().unwrap() {
        let c = &row["input"];
        let args = action::eligibility_arguments(
            c["autonomous"] == 1,
            c["kind"].as_u64().unwrap() as u32,
            c["third"].as_i64().unwrap() as i16,
            c["check"].as_u64().unwrap() as u16,
        );
        assert_eq!(json!(args), row["original"], "{c}");
    }
}
