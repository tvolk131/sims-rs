use sims_vm::loading::{HouseContext, SimulatorContext, house_globals};

#[cfg(feature = "private-fixtures")]
#[test]
fn selection_notification_matches_native_house_listener() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/house-selection-signal.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 72);
    for row in cases {
        let mut world = sims_vm::World::default();
        world.globals = (0..64).map(|i| (i, i * 17 - 200)).collect();
        world.globals.insert(3, row["old"].as_i64().unwrap() as i16);
        let mut expected = world.clone();
        expected
            .globals
            .insert(3, row["selected"].as_i64().unwrap() as i16);
        world
            .notify_person_selection(row["payload"].as_i64().unwrap() as i32)
            .unwrap();
        assert_eq!(world, expected, "{row}");
        assert_eq!(row["unchanged_other_bytes"], true);
    }
    let mut missing = sims_vm::World::default();
    let before = missing.clone();
    assert!(missing.notify_person_selection(195).is_err());
    assert_eq!(missing, before);
}

#[test]
fn installation_context_replaces_saved_family_and_budget_mode() {
    let mut saved = vec![0; 64];
    saved[9] = 7;
    saved[10] = 5;
    saved[20] = 1;
    saved[32] = 1;
    saved[33] = 0;
    saved[13] = 2542;
    let g = house_globals(
        &saved,
        HouseContext {
            simulator: SimulatorContext {
                game_mode: 0,
                retained_global25: 0,
                global30: 1,
            },
            installed_expansions: 255,
            performance_class: 2,
        },
    )
    .unwrap();
    assert_eq!([g[&9], g[&10], g[&20], g[&32], g[&33]], [0, 5, 255, 0, 2]);
    assert_eq!(
        g[&13], 2542,
        "the outer loader does not select the family budget yet"
    );
    assert_eq!(saved[9], 7);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_house_open_selects_family_and_uses_post_reload_coordinates() {
    // The native probe begins at 428041, with constructor-owned storage and a
    // supplied neighborhood-reload boundary. It stops before object activation.
    // This compares successful selection/order, not the missing coordinate
    // producer or a Rust implementation of the wrapper's failure lifecycle.
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/house-nearby-context.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 66);
    let mut compared = 0;
    for row in cases {
        let c = &row["input"];
        let original = &row["original"];
        if original["reached_activation"] != true {
            continue;
        }
        let coordinates: Vec<[i32; 2]> = serde_json::from_value(
            if c["reload_coordinates"].is_null() {
                &c["coordinates"]
            } else {
                &c["reload_coordinates"]
            }
            .clone(),
        )
        .unwrap();
        let house = c["house"].as_i64().unwrap() as i16;
        let family =
            if house != 0 && (c["family_house"] == house || c["family_secondary_house"] == house) {
                42
            } else {
                0
            };
        let mut world = sims_vm::World::default();
        world.globals = [(9, 321), (10, 654), (32, 0)].into();
        world.budget = Some(sims_vm::budget::State {
            balance: 9876,
            expenses: [0; 9],
            notifications: vec![],
        });
        world.person = Some(sims_vm::person::WorldState::default());
        world.select_loaded_family(family, house, 2542).unwrap();
        let expected = &original["final"];
        assert_eq!(world.person.as_ref().unwrap().controlled_family, family);
        assert_eq!(expected["selected_family"], family);
        assert_eq!(expected["selected_lot"], house);
        assert_eq!(
            serde_json::json!([world.globals[&9], world.globals[&10]]),
            expected["globals"],
            "{c}"
        );
        assert_eq!(expected["funds"], world.budget.as_ref().unwrap().balance);
        assert_eq!(
            serde_json::json!(sims_vm::nearby_houses::order(&coordinates, house.into()).unwrap()),
            expected["order"],
            "{c}"
        );
        let signal = original["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["service"] == "signal")
            .unwrap();
        assert_eq!(signal["kind"], 0xda);
        assert_eq!(signal["state"]["order"], serde_json::json!([]));
        assert_eq!(signal["state"]["globals"], expected["globals"]);
        assert_eq!(
            world.budget.as_ref().unwrap().notifications,
            [signal["value"].as_i64().unwrap() as i32]
        );
        compared += 1;
    }
    assert_eq!(compared, 18);
}

#[test]
fn opening_funds_republication_keeps_family_and_repeats_the_notice() {
    //The unchanged Newbie load calls46d8dc(665) from428340 and again
    //from4f23c7. The second setter must not be deduplicated or reselect the
    //family. Signed word splitting and the mode gate follow46d8dc.
    let mut world = sims_vm::World::default();
    world.globals = [(9, 1), (10, 7), (13, 4950), (14, 0), (32, 0)].into();
    world.budget = Some(sims_vm::budget::State {
        balance: 4950,
        expenses: [7; 9],
        notifications: vec![],
    });
    world.publish_family_funds(665).unwrap();
    world.publish_family_funds(665).unwrap();
    assert_eq!(world.budget.as_ref().unwrap().notifications, [665, 665]);
    assert_eq!([world.globals[&9], world.globals[&10]], [1, 7]);
    assert_eq!([world.globals[&13], world.globals[&14]], [665, 0]);
    assert_eq!(world.budget.as_ref().unwrap().expenses, [7; 9]);
    world.publish_family_funds(-12345).unwrap();
    assert_eq!([world.globals[&13], world.globals[&14]], [-2345, -1]);
    world.globals.insert(32, 1);
    let before = world.clone();
    world.publish_family_funds(9876).unwrap();
    assert_eq!(world, before);
    world.globals.insert(32, 0);
    world.budget.as_mut().unwrap().notifications.resize(4096, 0);
    let before = world.clone();
    assert!(world.publish_family_funds(9876).is_err());
    assert_eq!(world, before);
}

#[test]
#[ignore = "requires private original-code outer load-context oracle"]
fn original_outer_house_load_globals_and_order() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-context.json");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let saved: Vec<i16> = serde_json::from_value(c["globals"].clone()).unwrap();
        let g = house_globals(
            &saved,
            HouseContext {
                simulator: SimulatorContext {
                    game_mode: saved[15] as i8,
                    retained_global25: saved[25],
                    global30: saved[30] as u8,
                },
                installed_expansions: c["installed_expansions"].as_u64().unwrap() as u16,
                performance_class: c["performance_class"].as_i64().unwrap() as i16,
            },
        )
        .unwrap();
        // The probe isolates the enclosing wrapper. Inner SIMI speed/month
        // transforms have their own original-code comparison.
        let mut expected: Vec<i16> =
            serde_json::from_value(row["original"]["globals"].clone()).unwrap();
        expected[18] = expected[16];
        expected[29] = 30;
        assert_eq!(g.values().copied().collect::<Vec<_>>(), expected);
        let events = row["original"]["events"].as_array().unwrap();
        assert_eq!(events[0], "SIMI");
        assert_eq!(
            events[1],
            serde_json::json!(["size", if saved[23] == 0 { 64 } else { saved[23] }])
        );
        assert_eq!(events[2][0], "HOUS");
        assert_eq!(events[2][1][9], saved[9], "family clears after HOUS");
        assert_eq!(events[3], serde_json::json!(["family", 0]));
        assert_eq!(
            &events[4..8],
            &serde_json::json!(["clear_registry", "arrays", "floors", "walls"])
                .as_array()
                .unwrap()[..]
        );
        assert_eq!(events[8][0], "objects");
        assert_eq!(events[8][1], row["original"]["globals"]);
        assert_eq!(events[9], "rooms");
    }
}
