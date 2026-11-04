use sims_content::{
    Index, Resource,
    neighborhood_input::{InstalledHouses, MAX_LOCATOR_BYTES, MAX_MATCHING_HOUSES},
};
use std::path::PathBuf;

const HOUSE: &str = "UserData2/Houses/House09.iff";
const NEIGHBORHOOD: &str = "UserData2/Neighborhood.iff";

fn index(locators: &[&str]) -> Index {
    Index {
        root: PathBuf::from("/not-read"),
        resources: locators
            .iter()
            .map(|locator| Resource {
                locator: (*locator).into(),
                name: locator.rsplit('/').next().unwrap().into(),
                source: PathBuf::from("/not-read"),
                offset: 0,
                size: 0,
                unpacked_size: 0,
                archived: false,
                owned: None,
            })
            .collect(),
        archives: 0,
        diagnostics: vec![],
    }
}
fn discover(index: &Index) -> anyhow::Result<InstalledHouses> {
    InstalledHouses::discover(index, HOUSE, NEIGHBORHOOD, 9)
}

#[test]
fn installed_slots_are_numeric_namespace_local_and_include_vacant_looking_slots() {
    let mut inventory = index(&[
        "UserData2/Houses/House11.iff",
        "UserData2/Houses/House00.iff",
        "userdata2/hOuSeS/hOuSe01.IfF",
        HOUSE,
        "UserData2/Houses/House10.iff",
        "UserData/Houses/House03.iff",
        "UserData3/Houses/House04.iff",
        "TemplateUserData/Houses/House05.iff",
        "UserData2/Houses/Nested/House06.iff",
        "UserData2/Houses/Bundle.far::House07.iff",
        "UserData2/Houses/House08.iff",
    ]);
    inventory.resources.last_mut().unwrap().archived = true;
    let found = discover(&inventory).unwrap();
    assert_eq!(found.nearby_houses(), &[1, 10, 11]);
    assert_eq!(
        found
            .candidates()
            .iter()
            .map(|candidate| candidate.lot())
            .collect::<Vec<_>>(),
        [0, 1, 9, 10, 11]
    );
    assert_eq!(
        found.candidates()[1].locator(),
        "userdata2/hOuSeS/hOuSe01.IfF"
    );
    inventory.resources.reverse();
    assert_eq!(discover(&inventory).unwrap(), found);
    found.validate(HOUSE, NEIGHBORHOOD, 9).unwrap();
}

#[test]
fn synthetic_mashuga_named_inventory_has_the_proposed_order_not_observation_evidence() {
    let mut inventory = index(&[HOUSE]);
    for lot in (0..=11).rev().filter(|lot| *lot != 9) {
        inventory
            .resources
            .extend(index(&[&format!("UserData2/Houses/House{lot:02}.iff")]).resources);
    }
    assert_eq!(
        discover(&inventory).unwrap().nearby_houses(),
        &[1, 2, 3, 4, 5, 6, 7, 8, 10, 11]
    );
}

#[test]
fn malformed_names_are_ignored_but_matching_out_of_range_digits_reject() {
    let mut inventory = index(&[
        HOUSE,
        "UserData2/Houses/House32767.iff",
        "UserData2/Houses/House.iff",
        "UserData2/Houses/House-1.iff",
        "UserData2/Houses/House+1.iff",
        "UserData2/Houses/House１.iff",
        "UserData2/Houses/House1x.iff",
        "UserData2/Houses/House 1.iff",
        "UserData2/Houses/House1.iff.bak",
        "UserData2/Houses/House2/House3.iff",
        "UserData2/Houses/House2\\House3.iff",
        "UserData2/Houses/House1.iff/House2.iff",
    ]);
    assert_eq!(discover(&inventory).unwrap().nearby_houses(), &[32767]);
    for name in ["House32768.iff", "House999999999999999999999999.iff"] {
        inventory
            .resources
            .extend(index(&[&format!("UserData2/Houses/{name}")]).resources);
        assert!(discover(&inventory).is_err(), "accepted {name}");
        inventory.resources.pop();
    }
}

#[test]
fn duplicates_and_numeric_aliases_reject_even_for_excluded_slots() {
    for (first, second) in [
        ("House01.iff", "house01.IFF"),
        ("House1.iff", "House01.iff"),
        ("House00.iff", "House0.iff"),
        ("House09.iff", "House9.iff"),
        ("House09.iff", "HOUSE09.IFF"),
    ] {
        let mut inventory = index(&[HOUSE, "UserData2/Houses/House11.iff"]);
        if first != "House09.iff" {
            inventory
                .resources
                .extend(index(&[&format!("UserData2/Houses/{first}")]).resources);
        }
        inventory
            .resources
            .extend(index(&[&format!("UserData2/Houses/{second}")]).resources);
        assert!(discover(&inventory).is_err(), "accepted {first}/{second}");
    }
}

#[test]
fn current_identity_and_nonempty_output_are_required() {
    for inventory in [
        index(&[]),
        index(&[HOUSE]),
        index(&[HOUSE, "UserData2/Houses/House0.iff"]),
    ] {
        assert!(discover(&inventory).is_err());
    }
    let inventory = index(&[HOUSE, "UserData2/Houses/House01.iff"]);
    for (house, neighborhood, lot) in [
        (HOUSE, NEIGHBORHOOD, 8),
        (HOUSE, NEIGHBORHOOD, 0),
        (HOUSE, "UserData/Neighborhood.iff", 9),
        ("UserData2/Houses/Nested/House09.iff", NEIGHBORHOOD, 9),
        ("UserData2/Houses/House9.iff", NEIGHBORHOOD, 9),
        (
            "UserData0/Houses/House09.iff",
            "UserData0/Neighborhood.iff",
            9,
        ),
        (
            "UserData65536/Houses/House09.iff",
            "UserData65536/Neighborhood.iff",
            9,
        ),
        (
            "UserData+2/Houses/House09.iff",
            "UserData+2/Neighborhood.iff",
            9,
        ),
    ] {
        assert!(InstalledHouses::discover(&inventory, house, neighborhood, lot).is_err());
    }
    let original = index(&["UserData/Houses/House05.iff", "UserData/Houses/House01.iff"]);
    assert_eq!(
        InstalledHouses::discover(
            &original,
            "UserData/Houses/House05.iff",
            "UserData/Neighborhood.iff",
            5
        )
        .unwrap()
        .nearby_houses(),
        &[1]
    );
}

#[test]
fn matching_manifest_and_locator_bounds_are_enforced() {
    let mut inventory = index(&[]);
    for lot in 0..MAX_MATCHING_HOUSES {
        inventory
            .resources
            .extend(index(&[&format!("UserData2/Houses/House{lot:02}.iff")]).resources);
    }
    assert!(discover(&inventory).is_ok());
    inventory
        .resources
        .extend(index(&[&format!("UserData2/Houses/House{MAX_MATCHING_HOUSES}.iff")]).resources);
    assert!(discover(&inventory).is_err());
    let long = format!(
        "UserData2/Houses/House{}1.iff",
        "0".repeat(MAX_LOCATOR_BYTES)
    );
    assert!(discover(&index(&[HOUSE, &long])).is_err());
}

#[test]
fn retained_manifest_rejects_order_locators_and_missing_current_tampering() {
    let found = discover(&index(&[
        HOUSE,
        "UserData2/Houses/House01.iff",
        "UserData2/Houses/House11.iff",
    ]))
    .unwrap();
    let original = serde_json::to_value(&found).unwrap();
    for damaged in [
        serde_json::json!({"candidates": original["candidates"], "order": [11, 1]}),
        serde_json::json!({"candidates": original["candidates"], "order": []}),
        serde_json::json!({"candidates": [original["candidates"][0], original["candidates"][2]], "order": [1, 11]}),
    ] {
        let retained: InstalledHouses = serde_json::from_value(damaged).unwrap();
        assert!(retained.validate(HOUSE, NEIGHBORHOOD, 9).is_err());
    }
    for (key, value) in [
        ("locator", serde_json::json!("UserData/Houses/House01.iff")),
        ("lot", serde_json::json!(-1)),
    ] {
        let mut damaged = original.clone();
        damaged["candidates"][0][key] = value;
        let retained: InstalledHouses = serde_json::from_value(damaged).unwrap();
        assert!(retained.validate(HOUSE, NEIGHBORHOOD, 9).is_err());
    }
}
