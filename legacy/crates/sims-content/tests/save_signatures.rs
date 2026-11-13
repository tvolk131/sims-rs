use serde_json::{Value, json};
use sims_formats::behavior::Behavior;
use sims_vm::{Module, Program, Routine};
use std::{fs, path::PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn tree(p: &mut Program, id: u16, version: u32, opcodes: &[u16]) {
    let source = if id < 4096 {
        "global"
    } else if id < 8192 {
        "private"
    } else {
        "semi"
    };
    let module = p.modules.entry(source.into()).or_insert_with(|| Module {
        source: source.into(),
        semiglobal: Some("semi".into()),
        ..Default::default()
    });
    let mut data = vec![2, 128];
    data.extend_from_slice(&(opcodes.len() as u16).to_le_bytes());
    data.extend_from_slice(&[0, 4, 4, 0]);
    data.extend_from_slice(&version.to_le_bytes());
    for &op in opcodes {
        data.extend_from_slice(&op.to_le_bytes());
        data.extend_from_slice(&[254, 255, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    module.routines.insert(
        id,
        Routine {
            name: id.to_string(),
            body: Behavior::parse(&data).unwrap(),
        },
    );
    module.routine_versions.insert(id, version);
}

#[test]
fn signature_keeps_the_original_private_context_through_a_global_cycle() {
    let mut p = Program {
        globals: "global".into(),
        ..Default::default()
    };
    tree(&mut p, 4096, 7, &[4097, 0x9001, 9999, 2]);
    tree(&mut p, 4097, 11, &[4096, 300]);
    tree(&mut p, 300, 13, &[4097]);
    tree(&mut p, 8192, u32::MAX, &[]);
    assert_eq!(p.behavior_signature("private", 4096).unwrap(), 0x0007001f);
    assert_eq!(p.behavior_signature("private", 0).unwrap(), 0);
    assert_eq!(p.behavior_signature("private", 4098).unwrap(), 0);
    p.modules
        .get_mut("private")
        .unwrap()
        .routine_versions
        .remove(&4097);
    assert!(p.behavior_signature("private", 4096).is_err());
}

#[test]
#[ignore = "requires private original-code signature oracle"]
fn saved_behavior_signatures_match_original_traversal() {
    let report: Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/save-signatures.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for case in report["cases"].as_array().unwrap() {
        let mut p = Program {
            globals: "global".into(),
            ..Default::default()
        };
        p.modules.insert(
            "private".into(),
            Module {
                source: "private".into(),
                semiglobal: Some("semi".into()),
                ..Default::default()
            },
        );
        for (id, t) in case["input"]["trees"].as_object().unwrap() {
            let ops: Vec<u16> = serde_json::from_value(t["opcodes"].clone()).unwrap();
            tree(
                &mut p,
                id.parse().unwrap(),
                t["version"].as_u64().unwrap() as u32,
                &ops,
            );
        }
        let got = p
            .behavior_signature("private", case["input"]["root"].as_u64().unwrap() as u16)
            .unwrap();
        assert_eq!(
            got,
            case["original"]["signature"].as_u64().unwrap() as u32,
            "{case}"
        );
    }
}

#[test]
#[ignore = "requires private original house/object files"]
fn original_house_compatibility_inventory_uses_loaded_callback_graphs() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let mut reports = Vec::new();
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let p = house.link_program(&index).unwrap();
        let report = house.compatibility(&p).unwrap();
        let storage = house.prepare_storage(&p).unwrap();
        let mut execution = house.prepare_execution(&p).unwrap();
        assert_eq!(execution.vm.threads.len(), house.objects.instances.len());
        assert_eq!(execution.vm.tick, u64::from(house.simulator.ticks));
        assert_eq!(execution.vm.rng_state, house.simulator.global_rng);
        assert_eq!(execution.vm.lot_rng_state, Some(house.simulator.lot_rng));
        assert_eq!(execution.vm.crt_rng_state, None);
        assert_eq!(storage.world.objects.len(), house.objects.instances.len());
        assert_eq!(
            storage.world.person.as_ref().unwrap().object_order,
            house
                .objects
                .order
                .iter()
                .map(|&id| id as i16)
                .collect::<Vec<_>>()
        );
        for saved in house.objects.instances.values() {
            let object = &storage.world.objects[&(saved.id as i16)];
            assert_eq!(object.native_flags, Some(0x280));
            assert_eq!(object.temps, saved.temps.as_slice());
            assert_eq!(
                object.data.values().copied().collect::<Vec<_>>(),
                saved.data
            );
            assert_eq!(
                &object.attributes[..saved.attributes.len()],
                saved.attributes.as_slice()
            );
            if let Some(person) = &saved.person {
                assert!(object.slots.len() >= sims_vm::person::BUILTIN_SLOT_HEIGHTS.len());
                assert_eq!(object.person.as_ref().unwrap().len(), 256);
                assert_eq!(object.motives, Some(person.motives.map(f32::to_bits)));
                assert_eq!(
                    object.motive_update.as_ref().unwrap().previous,
                    person.previous_motives.map(f32::to_bits)
                );
                for (i, &word) in person.data.iter().enumerate() {
                    assert_eq!(object.person.as_ref().unwrap()[&(i as i16)], word);
                }
            }
        }
        execution
            .select_household(
                &house,
                sims_vm::loading::HouseContext {
                    simulator: sims_vm::loading::SimulatorContext {
                        game_mode: 0,
                        retained_global25: 0,
                        global30: 1,
                    },
                    installed_expansions: 0xff,
                    performance_class: 2,
                },
            )
            .unwrap();
        let clock = execution.lot_clock(&house).unwrap();
        assert_eq!(clock.ticks, house.simulator.ticks);
        assert_eq!(
            clock.statistics[5],
            house.simulator.budget_days[0].unwrap_or([0; 8])
        );
        for i in 0..5 {
            assert_eq!(
                clock.statistics[i],
                house.simulator.budget_days[i + 1].unwrap_or([0; 8])
            );
        }
        let family = house.household.as_ref().unwrap();
        assert_eq!(
            execution.vm.world.budget.as_ref().unwrap().balance,
            family.family.budget
        );
        assert_eq!(execution.vm.world.globals[&9], family.id as i16);
        assert_eq!(execution.vm.world.globals[&10], house.number);
        for resident in &family.residents {
            let id = resident.object as i16;
            let before = execution.vm.world.objects[&id]
                .person
                .as_ref()
                .unwrap()
                .clone();
            let motives = execution.vm.world.objects[&id].motives;
            execution.attach_resident_neighbor(&house, id).unwrap();
            let after = execution.vm.world.objects[&id].person.as_ref().unwrap();
            let neighbor = house
                .neighbors
                .entries
                .iter()
                .flatten()
                .find(|n| n.id == resident.neighbor)
                .unwrap();
            let copied = sims_vm::loading::persistent_person_fields(neighbor.person_mode);
            for field in 0..256 {
                let expected = if field == 31 {
                    resident.neighbor
                } else if field == 61 || copied.contains(&field) {
                    neighbor
                        .person_data
                        .get(field as usize)
                        .copied()
                        .unwrap_or(0)
                } else {
                    before[&field]
                };
                assert_eq!(
                    after[&field], expected,
                    "{source} resident {id} field {field}"
                );
            }
            assert_eq!(execution.vm.world.objects[&id].motives, motives);
        }
        if house.number == 9 {
            let a = execution.vm.world.actions.as_ref().unwrap();
            assert_eq!(a.serial, 119);
            assert_eq!(a.actors[&195].previous.as_ref().unwrap().target, 167);
        }
        println!(
            "{source}: {} modules, {} / {} changed types",
            p.modules.len(),
            report.iter().filter(|r| r.changed).count(),
            report.len()
        );
        let resident_guids: Vec<_> = house
            .household
            .as_ref()
            .unwrap()
            .residents
            .iter()
            .map(|r| r.guid)
            .collect();
        for guid in resident_guids {
            let row = report.iter().find(|r| r.guid == guid).unwrap();
            assert!(row.source.contains("/Characters/"));
            assert_ne!(row.main, 0);
        }
        reports.push(json!({"source":source,"types":report,"post_load":storage.post_load,"unresolved":storage.unresolved}));
    }
    fs::write(
        repo().join("cache/original-save-compatibility.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires private original-code post-load oracle"]
fn original_post_load_callback_and_reset_order() {
    let report: Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/save-activation.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let p = sims_vm::loading::PostLoad::new(
            c["kind"].as_u64().unwrap() as u16,
            c["changed"].as_bool().unwrap(),
            c["enabled"].as_bool().unwrap(),
            c["placeholder"].as_bool().unwrap(),
        );
        let level = c["level"].as_u64().unwrap().max(1);
        let mut events = vec![json!(["placement", level, 1])];
        if p.load_callback {
            events.push(json!(["callback", 2, 0, 0]));
        }
        if p.reset {
            events.extend([json!(["notify_people"]), json!(["virtual_reset", 0])]);
        }
        if p.init_callback {
            events.push(json!(["callback", 0, 0, 0]));
        }
        events.push(json!([
            "scheduler",
            71,
            2,
            u8::from(c["scheduled"].as_i64().unwrap() != 0)
        ]));
        events.push(json!([
            "scheduler",
            71,
            8,
            u8::from(c["interactions"].as_bool().unwrap())
        ]));
        assert_eq!(
            events,
            *row["original"]["events"].as_array().unwrap(),
            "{c}"
        );
        assert_eq!(level, row["original"]["level"].as_u64().unwrap());
        assert_eq!(
            row["original"]["coarse_level"],
            if c["level"] == 0 { 1 } else { 7 }
        );
    }
}

#[test]
#[ignore = "requires private original-code neighbor attachment oracle"]
fn original_neighbor_attachment_keeps_lot_state_and_copies_persistent_fields() {
    let report: Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/neighbor-attachment.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let words = |v: &Value| {
            v.as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, v)| (i as i16, v.as_i64().unwrap() as i16))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let mut person = words(&c["before"]);
        sims_vm::loading::attach_neighbor(
            &mut person,
            &words(&c["neighbor"]),
            c["id"].as_i64().unwrap() as i16,
            c["revision"].as_i64().unwrap() as i32,
        )
        .unwrap();
        assert_eq!(
            person,
            words(&row["original"]),
            "revision {}",
            c["revision"]
        );
    }
}

#[test]
#[ignore = "requires private original neighborhoods, houses and object resources"]
fn original_house_loading_rebuilds_stale_neighbor_family_words() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let mut house = sims_content::saves::House::load(&index, source).unwrap();
        let program = house.link_program(&index).unwrap();
        let before = house.prepare_storage(&program).unwrap();
        for neighbor in house.neighbors.entries.iter_mut().flatten() {
            neighbor
                .person_data
                .resize(neighbor.person_data.len().max(62), 0);
            neighbor.person_data[61] = -12345;
        }
        // The original reload discards members without a neighbor, rather
        // than rejecting the entire neighborhood. Keep raw source records.
        let missing_guid = 0xfedcba98;
        assert!(
            !house
                .neighbors
                .entries
                .iter()
                .flatten()
                .any(|n| n.guid == missing_guid)
        );
        for family in house.families.values_mut() {
            family.members.push(missing_guid);
        }
        let after = house.prepare_storage(&program).unwrap();
        assert_eq!(
            before.world, after.world,
            "{source}: stale NBRS word survived reload"
        );
        assert!(
            house
                .neighbors
                .entries
                .iter()
                .flatten()
                .all(|n| n.person_data[61] == -12345),
            "reload must not mutate source records"
        );
        assert!(
            house
                .families
                .values()
                .all(|f| f.members.last() == Some(&missing_guid))
        );
        let people = after.world.person.as_ref().unwrap();
        let expected_order: &[i16] = if source.starts_with("UserData2/") {
            &[1, 0]
        } else {
            &[4, 3, 2, 5, 1, 0]
        };
        assert_eq!(people.family_order.as_deref(), Some(expected_order));
        for (&family, members) in &people.families {
            for member in members {
                assert_eq!(people.neighbors[member].person[&61], family);
            }
        }
        let linked: std::collections::BTreeSet<_> = people.families.values().flatten().collect();
        for (id, neighbor) in &people.neighbors {
            if !linked.contains(id) {
                assert_eq!(
                    neighbor.person[&61], 0,
                    "unassigned neighbor retained a stale family"
                );
            }
        }
        let mut execution = house.prepare_execution(&program).unwrap();
        let household = house.household.as_ref().unwrap();
        for resident in &household.residents {
            execution
                .attach_resident_neighbor(&house, resident.object as i16)
                .unwrap();
            assert_eq!(
                execution.vm.world.objects[&(resident.object as i16)]
                    .person
                    .as_ref()
                    .unwrap()[&61],
                household.id as i16,
                "post-load attachment copied the stale raw NBRS field"
            );
        }
        println!(
            "{source}: {} neighbor family fields rebuilt from {} original families",
            people.neighbors.len(),
            people.families.len()
        );
        house.families.remove(&0).unwrap();
        let without_default = house.prepare_storage(&program).unwrap();
        assert_eq!(
            without_default.world, before.world,
            "{source}: missing family zero was not constructed by reload"
        );
        assert!(
            !house.families.contains_key(&0),
            "raw records must stay unchanged"
        );
    }
}

#[test]
#[ignore = "requires private original-code load context oracle"]
fn original_simulator_overlay_and_selected_family_budget() {
    let report: Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/load-context.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let words: Vec<i16> = serde_json::from_value(c["globals"].clone()).unwrap();
        let globals = sims_vm::loading::simulator_globals(
            &words,
            sims_vm::loading::SimulatorContext {
                game_mode: c["game_mode"].as_i64().unwrap() as i8,
                retained_global25: c["retained_global25"].as_i64().unwrap() as i16,
                global30: c["global30"].as_u64().unwrap() as u8,
            },
        )
        .unwrap();
        assert_eq!(
            json!(globals.values().collect::<Vec<_>>()),
            row["original"]["globals"]
        );
        let mut world = sims_vm::World {
            globals,
            person: Some(Default::default()),
            budget: Some(sims_vm::budget::State {
                balance: 2518,
                ..Default::default()
            }),
            ..Default::default()
        };
        // Use the values already in the oracle's globals to isolate the funds
        // setter. The actual family/house selection is checked on the corpus.
        world
            .select_loaded_family(
                world.globals[&9],
                world.globals[&10],
                c["funds"].as_i64().unwrap() as i32,
            )
            .unwrap();
        assert_eq!(
            world.person.as_ref().unwrap().controlled_family,
            world.globals[&9]
        );
        assert_eq!(
            json!(world.globals.values().collect::<Vec<_>>()),
            row["original"]["selected_globals"]
        );
        assert_eq!(
            json!(world.budget.as_ref().unwrap().balance),
            row["original"]["selected_balance"]
        );
        let funds_signals: Vec<i32> = row["original"]["signals"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s[0] == 218)
            .map(|s| s[1].as_u64().unwrap() as u32 as i32)
            .collect();
        assert_eq!(world.budget.as_ref().unwrap().notifications, funds_signals);
    }
}
