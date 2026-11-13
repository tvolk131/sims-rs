use serde::Deserialize;
use sims_vm::{ObjectId, selection::cycle_family};

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Original,
}
#[derive(Deserialize)]
struct Input {
    people: Vec<Person>,
    members: Vec<u32>,
}
#[derive(Deserialize)]
struct Person {
    id: ObjectId,
    guid: u32,
    flags: u32,
}
#[derive(Deserialize)]
struct Original {
    selected: Option<ObjectId>,
    calls: Vec<ObjectId>,
    other_storage_preserved: bool,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_family_cycle_matches_original_instructions() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/picker-family-cycle.json")).unwrap();
    assert_eq!(fixture.cases.len(), 192);
    for (index, case) in fixture.cases.into_iter().enumerate() {
        let selected = case
            .input
            .people
            .iter()
            .find(|p| p.flags & 2 != 0)
            .map(|p| p.id);
        let actual = cycle_family(selected, &case.input.members, |guid| {
            case.input
                .people
                .iter()
                .find(|p| p.guid == guid)
                .map(|p| p.id)
        });
        assert_eq!(actual, case.original.selected, "case {index}");
        assert_eq!(
            actual.into_iter().collect::<Vec<_>>(),
            case.original.calls,
            "case {index}"
        );
        assert!(case.original.other_storage_preserved);
    }
}
