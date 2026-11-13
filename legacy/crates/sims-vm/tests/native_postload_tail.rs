use serde_json::{Value, json};
use sims_vm::{
    ObjectId,
    load_finalize::{Request, Tail},
    selection::View,
};
use std::collections::BTreeSet;

struct People {
    order: Vec<ObjectId>,
    flags: Vec<u32>,
}
impl View for People {
    fn person_order(&self) -> &[ObjectId] {
        &self.order
    }
    fn flags(&self, id: ObjectId) -> Result<u32, String> {
        Ok(self.flags[id as usize - 1])
    }
    fn script_id(&self, id: ObjectId) -> Result<i16, String> {
        Ok(id)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_postload_enable_selected_and_live_star_vector_survive_saves() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/postload-tail.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 96);
    for case in fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let mut view = People {
            order: serde_json::from_value(input["order"].clone()).unwrap(),
            flags: serde_json::from_value(input["flags"].clone()).unwrap(),
        };
        let mut enabled = input["enabled"].as_u64().unwrap() as u8;
        let mut tail = Tail::default();
        let mut events = vec![];
        let mut applied = BTreeSet::new();
        while let Some(request) = tail.advance(&mut enabled, &view).unwrap() {
            tail = serde_json::from_slice(&serde_json::to_vec(&tail).unwrap()).unwrap();
            let before = (tail.clone(), enabled);
            assert_eq!(tail.advance(&mut enabled, &view).unwrap(), Some(request));
            let wrong = if request == Request::Initialize {
                Request::Star(1)
            } else {
                Request::Initialize
            };
            assert!(tail.acknowledge(wrong).is_err());
            assert_eq!((tail.clone(), enabled), before);
            let call = match request {
                Request::Initialize => json!(["initialize", 0]),
                Request::Selection(id) => json!(["selection", id.unwrap_or(0)]),
                Request::Star(id) => json!(["star", id]),
            };
            events
                .push(json!({"call":call,"enabled":enabled,"order":view.order,"flags":view.flags}));
            for (i, change) in input["changes"].as_array().unwrap().iter().enumerate() {
                if change["at"] == call && applied.insert(i) {
                    if !change["enabled"].is_null() {
                        enabled = change["enabled"].as_u64().unwrap() as u8;
                    }
                    if !change["flags"].is_null() {
                        view.flags = serde_json::from_value(change["flags"].clone()).unwrap();
                    }
                    if !change["order"].is_null() {
                        view.order = serde_json::from_value(change["order"].clone()).unwrap();
                    }
                }
            }
            tail = serde_json::from_slice(&serde_json::to_vec(&tail).unwrap()).unwrap();
            tail.acknowledge(request).unwrap();
            assert!(tail.acknowledge(request).is_err());
        }
        assert!(tail.complete());
        assert_eq!(
            json!({"events":events,"applied":applied,"enabled":enabled,"order":view.order,"flags":view.flags}),
            case["original"],
            "{input}"
        );
    }
}
