use serde_json::{Value, json};
use sims_sim::pose::model_material::*;
use std::collections::BTreeMap;

fn patch(target: &mut Value, changes: &Value) {
    if let (Some(target), Some(changes)) = (target.as_object_mut(), changes.as_object()) {
        for (key, value) in changes {
            if value.is_object() && target.contains_key(key) {
                patch(target.get_mut(key).unwrap(), value);
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    } else {
        panic!("patch object required");
    }
}
fn restore(update: &mut Update, state: &mut State) {
    let before = (update.clone(), state.clone());
    (*update, *state) = serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
    assert_eq!((update.clone(), state.clone()), before);
    update.validate().unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn material_controls_match_original_publication_references_and_live_vector_reads() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-model-material.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 209);
    for (case, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut normal = c["normal_mode"].as_u64().unwrap() as u32;
        let mut update = Update::new(serde_json::from_value(c["requested"].clone()).unwrap());
        let mut events = vec![];
        let mut occurrences = BTreeMap::new();
        loop {
            restore(&mut update, &mut state);
            let Some(request) = update.advance(&mut state, normal).unwrap() else {
                break;
            };
            let before = (update.clone(), state.clone());
            assert_eq!(
                update.advance(&mut state, !normal).unwrap(),
                Some(request.clone())
            );
            assert_eq!((update.clone(), state.clone()), before);
            let kind = match request {
                Request::Cast { .. } => "Cast",
                Request::Acquire { .. } => "Acquire",
                Request::Publish { .. } => "Publish",
                Request::Release { .. } => "Release",
            };
            events.push(json!({"kind":kind,"request":request,"state":state,"normal_mode":normal}));
            assert_eq!(
                events.last().unwrap(),
                &row["original"]["events"][events.len() - 1],
                "case {case}, service {}",
                events.len() - 1
            );
            let reply = match request {
                Request::Cast { object } => {
                    Reply::Cast(object.filter(|id| state.drawables.contains_key(id)))
                }
                Request::Acquire { material } => {
                    state.acquire(material).unwrap();
                    Reply::Done
                }
                Request::Publish { material, words } => {
                    assert_eq!(state.materials[&material].words, words);
                    Reply::Done
                }
                Request::Release { material } => {
                    state.release_retained(material).unwrap();
                    Reply::Done
                }
            };
            let occurrence = occurrences.entry(kind).or_insert(0_u64);
            for mutation in c["mutations"].as_array().unwrap() {
                if mutation["after"] == kind
                    && mutation["occurrence"].as_u64().unwrap_or(0) == *occurrence
                {
                    let mut changes = mutation["patch"].clone();
                    if let Some(value) = changes.as_object_mut().unwrap().remove("normal_mode") {
                        normal = value.as_u64().unwrap() as u32;
                    }
                    let mut value = serde_json::to_value(&state).unwrap();
                    patch(&mut value, &changes);
                    state = serde_json::from_value(value).unwrap();
                }
            }
            *occurrence += 1;
            restore(&mut update, &mut state);
            update.acknowledge(&request, reply).unwrap();
            assert!(update.acknowledge(&request, Reply::Done).is_err());
        }
        assert!(update.complete());
        assert_eq!(
            json!({"events":events,"state":state,"normal_mode":normal}),
            row["original"],
            "case {case}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn final_reference_requires_explicit_destruction_and_keeps_the_request_pending() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-model-material.json")).unwrap();
    let mut state: State =
        serde_json::from_value(fixture["cases"][2]["input"]["state"].clone()).unwrap();
    let mut update = Update::new([0; 3]);
    loop {
        let request = update.advance(&mut state, 0).unwrap().unwrap();
        let reply = match request {
            Request::Cast { object } => {
                Reply::Cast(object.filter(|id| state.drawables.contains_key(id)))
            }
            Request::Acquire { material } => {
                state.acquire(material).unwrap();
                Reply::Done
            }
            Request::Publish { .. } => Reply::Done,
            Request::Release { material } => {
                state.materials.get_mut(&material).unwrap().references = 1;
                let before = state.clone();
                assert!(
                    state
                        .release_retained(material)
                        .unwrap_err()
                        .contains("destructor")
                );
                assert_eq!(state, before);
                restore(&mut update, &mut state);
                assert_eq!(update.advance(&mut state, 0).unwrap(), Some(request));
                break;
            }
        };
        update.acknowledge(&request, reply).unwrap();
    }
}
