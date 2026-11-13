use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Original,
}
#[derive(Deserialize, Debug)]
struct Input {
    profile: Vec<Option<String>>,
    types: Vec<String>,
}
#[derive(Deserialize)]
struct Original {
    person: Vec<i16>,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn appearance_profile_matches_original_field_writes() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/person-profile.json")).unwrap();
    assert_eq!(fixture.cases.len(), 216);
    for case in fixture.cases {
        let mut person: BTreeMap<_, _> = (0..256).map(|i| (i, 100 + i)).collect();
        sims_vm::person_reset::reload_profile_fields(
            &mut person,
            &case
                .input
                .profile
                .iter()
                .map(|s| s.as_deref())
                .collect::<Vec<_>>(),
            &case
                .input
                .types
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            person.values().copied().collect::<Vec<_>>(),
            case.original.person,
            "{:?}",
            case.input
        );
    }
}
