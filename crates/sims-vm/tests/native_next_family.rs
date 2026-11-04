use serde_json::Value;
use sims_vm::{
    Object, World,
    person::{self, WorldState},
};
#[cfg(feature = "private-fixtures")]
#[test]
fn cyclic_same_family_person_iterator_matches_original() {
    let report: Value = serde_json::from_str(include_str!("fixtures/next-family.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 84);
    for row in report["cases"].as_array().unwrap() {
        let mut world = World::default();
        for (i, family) in row["input"]["families"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            world.objects.insert(
                i as i16 + 1,
                Object {
                    person: (i < 3).then(|| [(61, family.as_i64().unwrap() as i16)].into()),
                    ..Default::default()
                },
            );
        }
        world.person = Some(WorldState {
            person_order: serde_json::from_value(row["input"]["order"].clone()).unwrap(),
            ..Default::default()
        });
        let current = row["input"]["current"].as_i64().unwrap() as i16;
        let result = person::next_family_member(&world, current);
        if row["original"]["error"].is_null() {
            let next = result.unwrap();
            assert_eq!(
                next.is_some(),
                row["original"]["returned"].as_bool().unwrap(),
                "{row}"
            );
            assert_eq!(
                next.unwrap_or(current),
                row["original"]["value"].as_i64().unwrap() as i16,
                "{row}"
            );
        } else {
            assert_eq!(row["original"]["error"], 28);
            assert!(result.unwrap_err().contains("native error 0x1c"));
        }
    }
}
