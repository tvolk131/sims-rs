use serde_json::Value;
use sha2::{Digest, Sha256};
use sims_formats::{field::Fields, iff::Iff, lot, neighborhood};
use std::{fs, path::PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "requires private original neighborhoods and native_neighborhood_read.py results"]
fn original_neighborhood_storage_matches_native_reader() {
    let report: Value = serde_json::from_slice(
        &fs::read(repo().join("cache/native-neighborhood-read-original-files.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let path = repo()
            .join("game-data/cabinet-extracted")
            .join(case["source"].as_str().unwrap());
        let iff = Iff::parse(fs::read(path).unwrap()).unwrap();
        let bytes = iff.data(iff.get(b"NGBH", 1).unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            case["payload_sha256"]
        );
        let loaded = neighborhood::Neighborhood::parse(bytes).unwrap();
        assert_eq!(serde_json::json!(loaded.data), case["original"]["data"]);
        assert_eq!(
            serde_json::json!(loaded.options),
            case["original"]["options"]
        );
        assert_eq!(
            serde_json::json!(loaded.inventories),
            case["original"]["inventories"]
        );
        assert_eq!(
            bytes.len() - loaded.tail.len(),
            case["original"]["consumed"].as_u64().unwrap() as usize
        );
    }
}

#[test]
#[ignore = "requires private native save-codec oracle"]
fn original_field_and_array_codecs_match_native() {
    let report: Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/save-codecs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for case in report["fields"].as_array().unwrap() {
        let bytes: Vec<u8> = serde_json::from_value(case["data"].clone()).unwrap();
        let mut f = Fields::new(&bytes, case["origin"].as_u64().unwrap() as usize);
        let mut actual = Vec::new();
        for op in case["operations"].as_array().unwrap() {
            actual.push(match op.as_str().unwrap() {
                "short" => Value::from(f.i16().unwrap()),
                "int" => Value::from(f.i32().unwrap()),
                "byte" => Value::from(f.u8().unwrap()),
                "string" => Value::from(f.string().unwrap()),
                _ => unreachable!(),
            });
        }
        assert_eq!(actual, *case["original"]["values"].as_array().unwrap());
        assert_eq!(
            f.byte_position(),
            case["original"]["byte_cursor"].as_u64().unwrap() as usize
        );
    }
    for case in report["arrays"].as_array().unwrap() {
        let bytes: Vec<u8> = serde_json::from_value(case["data"].clone()).unwrap();
        let decoded = lot::Array::parse(&bytes).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&decoded.data)),
            case["original"]["sha256"]
        );
        assert_eq!(
            decoded.width,
            case["original"]["width"].as_u64().unwrap() as usize
        );
        assert_eq!(
            decoded.height,
            case["original"]["height"].as_u64().unwrap() as usize
        );
        assert_eq!(
            decoded.stride,
            case["original"]["stride"].as_u64().unwrap() as usize
        );
        assert_eq!(
            bytes.len(),
            case["original"]["byte_cursor"].as_u64().unwrap() as usize
        );
    }
}

#[test]
#[ignore = "requires private original UserData"]
fn original_newbie_family_retains_saved_members_funds_and_traits() {
    let root = repo().join("game-data/cabinet-extracted/UserData");
    let iff = Iff::parse(fs::read(root.join("Neighborhood.iff")).unwrap()).unwrap();
    let family = neighborhood::Family::parse(iff.data(iff.get(b"FAMI", 1).unwrap())).unwrap();
    assert_eq!((family.house, family.budget), (7, 665));
    assert_eq!(family.members, [0x32aa2056, 0x3251685c]);
    let neighbors = neighborhood::Neighbors::parse(iff.data(iff.get(b"NBRS", 1).unwrap())).unwrap();
    let betty = neighbors
        .entries
        .iter()
        .flatten()
        .find(|n| n.guid == 0x3251685c)
        .unwrap();
    let bob = neighbors
        .entries
        .iter()
        .flatten()
        .find(|n| n.guid == 0x32aa2056)
        .unwrap();
    assert_eq!(betty.person_data[3], 400);
    assert_eq!(bob.person_data[3], 400);
    assert_eq!(betty.id, 47);
    assert_eq!(bob.id, 48);
    assert!(!betty.relationships.is_empty());
    // Every truncation must either fail or preserve *all* records. Optional tail
    // bytes can be removed without changing the decoded family itself.
    let bytes = iff.data(iff.get(b"FAMI", 1).unwrap());
    for end in 0..bytes.len() {
        assert!(neighborhood::Family::parse(&bytes[..end]).is_err());
    }
}

#[test]
#[ignore = "requires private original houses and character resources"]
fn three_original_households_link_without_demo_defaults_or_cross_neighborhood_characters() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    for (source, count, funds, objects) in [
        ("UserData/Houses/House07.iff", 2, 665, 182),
        ("UserData/Houses/House05.iff", 3, 2542, 245),
        ("UserData2/Houses/House09.iff", 2, 7698, 205),
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        assert_eq!((house.width, house.height), (64, 64));
        assert_eq!(house.objects.instances.len(), objects);
        let household = house.household.as_ref().unwrap();
        assert_eq!(household.residents.len(), count);
        assert_eq!(household.family.budget, funds);
        // The Goth neighborhood budget differs from its lot snapshot. Preserve
        // both until native activation establishes which one is authoritative.
        assert_eq!(
            house.simulator.funds,
            if source.ends_with("House05.iff") {
                2515
            } else {
                funds
            }
        );
        assert_eq!(house.simulator.globals.len(), 32);
        assert!(house.simulator.ticks > 0);
        let prefix = source.split('/').next().unwrap().to_string() + "/Characters/";
        for resident in &household.residents {
            assert!(resident.character_source.starts_with(&prefix));
            assert!(!resident.name.starts_with("user"));
            let saved = &house.objects.instances[&resident.object];
            assert_eq!(saved.guid, resident.guid);
            let person = saved.person.as_ref().unwrap();
            assert!(person.data[3] > 0);
            assert!(person.motives.iter().any(|&m| m > 0. && m < 99.));
            assert!(!saved.stack.is_empty());
            assert_eq!(saved.data.len(), 72);
            assert!(saved.stack_capacity_bytes > 0);
        }
        // No instance is discarded simply because its tile has other objects.
        for (level, floor) in house.floors.iter().enumerate() {
            for (cell, &head) in floor.object_heads.iter().enumerate() {
                if head != 0 && !house.objects.instances.contains_key(&head) {
                    assert!(house.missing_tile_heads.iter().any(|d| d.object == head
                        && d.floor == level as i16 + 1
                        && d.x == cell / house.height
                        && d.y == cell % house.height));
                }
            }
        }
        if source.ends_with("House05.iff") {
            assert!(house.floors[1].coverings.iter().any(|&id| id != 0));
            assert!(household.residents.iter().any(|r| r.name == "Cassandra"));
        }
        if source == "UserData/Houses/House07.iff" {
            let bob = house.objects.instances[&105].person.as_ref().unwrap();
            assert_eq!(bob.motives[3], 74.07298);
            assert_eq!(bob.previous_motives[3], 57.385277);
            assert_eq!(house.house.name, "Kiss Me");
            assert_eq!(house.view.roof, "r1_");
        }
        println!(
            "{}: {} · {} objects · {} saved residents · §{} · {} stale tile heads",
            source,
            household.name,
            objects,
            count,
            funds,
            house.missing_tile_heads.len()
        );
    }
}

/// The explicit appearance saved for each real resident uses its own mesh,
/// texture and skeleton; this does not resume the original animator state.
#[test]
#[ignore = "requires original households and their saved avatar resources"]
fn seven_saved_resident_appearances_resolve_without_templates() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let mut seen = 0;
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        for resident in &house.household.as_ref().unwrap().residents {
            let saved = house.objects.instances[&resident.object]
                .person
                .as_ref()
                .unwrap();
            let avatar = sims_content::saved_avatar::load(&index, &resident.body_strings[0], saved)
                .unwrap_or_else(|e| panic!("{}: {e:#}", resident.name));
            assert_eq!(avatar.parts.len(), if saved.data[58] == 9 { 5 } else { 4 });
            assert_eq!(
                avatar.skeleton.name,
                if saved.data[58] == 9 {
                    "child"
                } else {
                    "adult"
                }
            );
            println!(
                "{}: {} skeleton, {} parts, {} source resources",
                resident.name,
                avatar.skeleton.name,
                avatar.parts.len(),
                avatar.sources.len()
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 7);
}

#[test]
#[ignore = "requires original houses; checks full ObjM registry words before callbacks"]
fn original_saved_registry_words_survive_deserialization() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let program = house.link_program(&index).unwrap();
        let execution = house.prepare_execution(&program).unwrap();
        execution.vm.world.validate_scheduler_storage().unwrap();
        let registry = execution.vm.world.scheduler_storage.as_ref().unwrap();
        assert_eq!(
            registry.len(),
            usize::from(*house.objects.instances.keys().max().unwrap())
        );
        for id in 1..=registry.len() as i16 {
            assert_eq!(
                registry.contains(id),
                house.objects.instances.contains_key(&(id as u16))
            );
            if !registry.contains(id) {
                assert_eq!(registry.word(&execution.vm.world, id).unwrap(), 0);
            } else {
                assert_eq!(
                    registry.object_for_handle(registry.handle(id).unwrap()),
                    Some(id)
                );
                assert_eq!(execution.vm.world.objects[&id].native_tree_error, Some(0));
            }
        }
        let mut active = 0;
        let mut advertised = 0;
        for (&id, saved) in &house.objects.instances {
            let state = execution.vm.world.objects[&(id as i16)]
                .idle
                .as_ref()
                .unwrap();
            assert_eq!(
                state.registry_word(),
                saved.registry_word as u32,
                "{source} object {id}"
            );
            active += usize::from(state.flags & 1 != 0);
            advertised += usize::from(state.flags & 8 != 0);
        }
        println!(
            "{source}: {} original words retained; {active} active, {advertised} advertising",
            house.objects.instances.len()
        );
    }
}
