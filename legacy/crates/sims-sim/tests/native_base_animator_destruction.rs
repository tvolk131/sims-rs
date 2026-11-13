use serde_json::{Value, json};
use sims_sim::pose::graphic_destruction::{Execution, Request, State, Storage};

fn compact(s: &State) -> Value {
    let records = |v: &[Option<u64>]| {
        v.iter()
            .map(|n| n.map(|i| format!("record{i}")))
            .collect::<Vec<_>>()
    };
    json!({"model":s.model.is_some(),"layers":s.layers,"auxiliary":records(&s.auxiliary),"generic":records(&s.generic)})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_animator_destruction_matches_native_with_saved_callback_mutations() {
    let report: Value = serde_json::from_str(include_str!(
        "fixtures/native-base-animator-destruction.json"
    ))
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 105);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut state = State {
            model: (c["model"] == true).then_some(1),
            layers: serde_json::from_value(c["layers"].clone()).unwrap(),
            auxiliary: serde_json::from_value(c["auxiliary"].clone()).unwrap(),
            generic: serde_json::from_value(c["generic"].clone()).unwrap(),
            focus: c["focus"] == true,
        };
        let mut e = Execution::default();
        let mut events = vec![];
        while let Some(r) = e.advance(&mut state).unwrap() {
            let (call, target, free) = match r {
                Request::Reset => ("reset", Some("0x10000000".to_owned()), false),
                Request::ClearAppearances => {
                    ("clear_appearances", Some("0x10000000".to_owned()), false)
                }
                Request::DetachLayer(id) => ("detach_layer", Some(format!("layer{id}")), false),
                Request::DestroyModel(_) => ("destroy_model", Some("model".into()), true),
                Request::ClearFocus => ("clear_base_focus", None, false),
                Request::DestroyRecord { record, .. } => {
                    ("destroy_record", Some(format!("record{record}")), true)
                }
                Request::Storage(Storage::Name) => ("destroy_name", Some("name".into()), false),
                Request::Storage(s) => (
                    "destroy_vector",
                    Some(
                        match s {
                            Storage::Auxiliary => "0x100000cc",
                            Storage::Generic => "0x100000c0",
                            Storage::Layers => "0x1000002c",
                            _ => unreachable!(),
                        }
                        .into(),
                    ),
                    false,
                ),
            };
            events.push(json!({"call":call,"target":target,"state":compact(&state)}));
            if c["mutation"]["after"] == call {
                state.layers = serde_json::from_value(c["mutation"]["layers"].clone()).unwrap();
            }
            if free {
                events.push(json!({"call":"free","target":target,"state":compact(&state)}));
            }
            // The pending request retains its captured pointer while future
            // fields may differ. Restoring must not reread/reissue the call.
            e = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
            e.validate(&state).unwrap();
            assert_eq!(e.advance(&mut state).unwrap(), Some(r));
            e.acknowledge(&mut state, r).unwrap();
        }
        assert!(e.complete());
        assert_eq!(
            json!({"events":events,"state":compact(&state)}),
            row["original"],
            "{c}"
        );
    }
}
