use serde_json::{Value, json};
use sims_vm::{
    person,
    person_factory::{self, InitialReset, InitialResetRequest as R, NeighborAttachment as N},
};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn constructor_reset_order_fields_rng_and_continuations_match_original() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/person-initial-reset.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 130);
    for row in rows {
        let c = &row["input"];
        let n = &row["original"];
        let slots: Vec<_> = c["slots"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, present)| (present == 1).then_some(i as i16 + 1))
            .collect();
        let mut neighborhood = person::WorldState {
            neighbor_slots: Some(slots.clone()),
            ..Default::default()
        };
        for id in slots.iter().flatten().copied() {
            neighborhood.neighbors.insert(
                id,
                person::Neighbor {
                    guid: if c["match"].as_i64() == Some(i64::from(id - 1)) {
                        0x44a86a6c
                    } else {
                        id as u32
                    },
                    person_revision: c["revision"].as_i64().unwrap() as i32,
                    person: (0..256).map(|i| (i, i + 1000)).collect(),
                    ..Default::default()
                },
            );
        }
        let initial_count = neighborhood.neighbors.len();
        let mut person: BTreeMap<_, _> = c["before"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, v)| (i as i16, v.as_i64().unwrap() as i16))
            .collect();
        let mut crt = c["seed"].as_u64().unwrap() as u32;
        let mut reset = InitialReset::begin(&mut person, c["update"] == 1).unwrap();
        let mut events = vec![];
        while let Some(request) = reset.request() {
            // Serialize the pending service and all mutable inputs, and verify
            // that restoring them yields exactly the same next request.
            let bytes = serde_json::to_vec(&(&reset, &person, &neighborhood, crt)).unwrap();
            let restored: (InitialReset, BTreeMap<i16, i16>, person::WorldState, u32) =
                serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                restored,
                (reset.clone(), person.clone(), neighborhood.clone(), crt)
            );
            (reset, person, neighborhood, crt) = restored;
            assert_eq!(reset.request(), Some(request));
            let name = match request {
                R::ReloadAppearance => "profile",
                R::AttachNeighbor(N::Persistent) => "neighbor",
                R::AttachNeighbor(N::Traits) => "traits",
                R::BaseReset { .. } => "base_reset",
                R::Init => "init",
                R::IsResidential => "residential",
                R::RandomizeTraits => "randomize",
                R::ClearAnimator => "clear_animator",
                R::LoadScores => "scores",
            };
            events.push(json!({"kind":name, "person":person.values().copied().collect::<Vec<_>>(), "crt":crt}));
            if let Some(writes) = c[name].as_object() {
                for (field, value) in writes {
                    person.insert(field.parse().unwrap(), value.as_i64().unwrap() as i16);
                }
            }
            match request {
                R::AttachNeighbor(mode) => {
                    neighborhood
                        .attach_person_neighbor(&mut person, 0x44a86a6c, mode)
                        .unwrap();
                }
                R::RandomizeTraits => {
                    person_factory::default_traits(&mut person, &mut crt).unwrap()
                }
                R::BaseReset { update } => assert_eq!(update, c["update"] == 1),
                _ => {}
            }
            reset
                .complete(
                    &mut person,
                    request,
                    (request == R::IsResidential).then(|| c["is_residential"].as_bool().unwrap()),
                )
                .unwrap();
        }
        assert_eq!(json!(events), n["events"], "{c}");
        assert_eq!(
            json!(person.values().copied().collect::<Vec<_>>()),
            n["person"],
            "{c}"
        );
        assert_eq!(json!(crt), n["crt"], "{c}");
        assert_eq!(json!(neighborhood.neighbor_slots), n["slots"]);
        assert_eq!(
            json!(neighborhood.neighbors.len() - initial_count),
            n["allocated"]
        );
    }
}

#[test]
fn mismatched_service_acknowledgement_cannot_advance_reset() {
    let mut person = (0..256).map(|i| (i, i + 100)).collect();
    let mut reset = InitialReset::begin(&mut person, true).unwrap();
    let before = (reset.clone(), person.clone());
    assert!(reset.complete(&mut person, R::Init, None).is_err());
    assert_eq!((reset.clone(), person.clone()), before);
    assert!(
        reset
            .complete(&mut person, R::ReloadAppearance, Some(true))
            .is_err()
    );
    assert_eq!((reset, person), before);
}
