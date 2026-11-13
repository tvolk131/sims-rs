use serde_json::{Value, json};
use sims_sim::pose::graphic_owner::*;

#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_graphic_owner_matches_native_publication_and_callback_mutations() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-owner.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 176);
    for (case, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut update = Update::new(7);
        let mut events = vec![];
        let mut failed = false;
        loop {
            let saved = (state.clone(), update.clone());
            (state, update) = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
            assert_eq!((state.clone(), update.clone()), saved);
            let request = match update.advance(&state) {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(e) if c["allocation_fails"] == true => {
                    assert!(e.contains("dereference null"));
                    failed = true;
                    break;
                }
                Err(e) => panic!("case {case}: {e}"),
            };
            let before = update.clone();
            assert_eq!(update.advance(&state).unwrap(), Some(request.clone()));
            assert_eq!(update, before);
            events.push(json!({"request":request,"state":state}));
            let name = match request {
                Request::Profile { .. } => "Profile",
                Request::Construct => "Construct",
                Request::Initialize { .. } => "Initialize",
            };
            for mutation in c["mutations"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["after"] == name)
            {
                let mut value = serde_json::to_value(&state).unwrap();
                for (k, v) in mutation["state"].as_object().unwrap() {
                    value[k] = v.clone();
                }
                state = serde_json::from_value(value).unwrap();
            }
            let reply = match request {
                Request::Profile { .. } => Reply::Profile,
                Request::Construct => Reply::Constructed(if c["allocation_fails"] == true {
                    None
                } else {
                    Some(1)
                }),
                Request::Initialize { .. } => {
                    Reply::Initialized(c["initialized_result"].as_u64().unwrap() as u8)
                }
            };
            update = serde_json::from_slice(&serde_json::to_vec(&update).unwrap()).unwrap();
            update.acknowledge(&mut state, &request, reply).unwrap();
            assert!(
                update
                    .acknowledge(&mut state, &request, Reply::Initialized(1))
                    .is_err()
            );
        }
        assert_eq!(update.complete(), !failed);
        assert_eq!(
            json!({"state":state,"events":events,"failed":failed}),
            row["original"],
            "case {case}"
        );
    }
}
