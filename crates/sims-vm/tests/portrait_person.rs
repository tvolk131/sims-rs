use serde::Deserialize;
use serde_json::{Value, json};
use sims_vm::image_cache::{
    Entry, State,
    temporary::{Request, Response, Temporary},
};

#[derive(Deserialize)]
struct Row {
    input: Value,
    original: Value,
}
#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Row>,
}
fn flag(c: &Value, key: &str) -> bool {
    c[key].as_bool().unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn temporary_person_matches_native_factory_type_placement_and_cleanup_order() {
    let f: Fixture = serde_json::from_str(include_str!("fixtures/portrait-person.json")).unwrap();
    assert_eq!(f.cases.len(), 924);
    for (i, row) in f.cases.into_iter().enumerate() {
        let c = row.input;
        let cache = State {
            entries: vec![Entry::new(c["guid"].as_u64().unwrap() as u32)],
            ..Default::default()
        };
        let original_cache = cache.clone();
        let mut run = Temporary::new(c["index"].as_u64().unwrap() as u32);
        let mut events = vec![];
        while let Some(request) = run.advance(&cache).unwrap() {
            let bytes = serde_json::to_vec(&run).unwrap();
            assert_eq!(run.advance(&cache).unwrap(), Some(request));
            assert_eq!(serde_json::to_vec(&run).unwrap(), bytes);
            run = serde_json::from_slice(&bytes).unwrap();
            let (event, response) = match request {
                Request::Resource { guid } => (
                    json!({"kind":"resource","guid":guid}),
                    Response::Resource(flag(&c, "resource").then_some(1)),
                ),
                Request::Create { resource } => (
                    json!({"kind":"create","resource":resource}),
                    Response::Created(c["created"].as_u64().unwrap() as u32),
                ),
                Request::Person { id } => (
                    json!({"kind":"person","id":id}),
                    Response::Person(
                        (id == 3 && flag(&c, "registered") && flag(&c, "person")).then_some(1),
                    ),
                ),
                Request::Find {
                    person,
                    descriptor,
                    output,
                } => (
                    json!({"kind":"find","person":person,"descriptor":descriptor,"output":output}),
                    Response::Found {
                        found: flag(&c, "found"),
                        position: serde_json::from_value(c["position"].clone()).unwrap(),
                        anchored_level: flag(&c, "anchored")
                            .then(|| c["level"].as_i64().unwrap() as i32),
                    },
                ),
                Request::Move {
                    person,
                    position,
                    level,
                    flags,
                } => (
                    json!({"kind":"move","person":person,"position":position,"level":level,"flags":flags}),
                    Response::Done,
                ),
                Request::Remove { created } => {
                    (json!({"kind":"remove","created":created}), Response::Done)
                }
            };
            events.push(event);
            run.respond(&cache, response).unwrap();
            run = serde_json::from_slice(&serde_json::to_vec(&run).unwrap()).unwrap();
            run.validate(&cache).unwrap();
        }
        let mut expected = row.original["events"].as_array().unwrap().clone();
        for event in &mut expected {
            if event["kind"] == "find" {
                assert_eq!(
                    event.as_object_mut().unwrap().remove("untouched").unwrap(),
                    json!(vec![165; 7])
                );
            }
        }
        assert_eq!(events, expected, "case {i}");
        assert_eq!(
            json!(run.returned().unwrap()),
            row.original["person"],
            "case {i}"
        );
        assert_eq!(
            cache, original_cache,
            "temporary creation must not use/update lazy resource cache"
        );
    }
}

#[test]
fn temporary_person_rejects_invalid_response_and_changed_catalog() {
    let mut cache = State {
        entries: vec![Entry::new(11)],
        ..Default::default()
    };
    let mut run = Temporary::new(0);
    assert_eq!(
        run.advance(&cache).unwrap(),
        Some(Request::Resource { guid: 11 })
    );
    let before = run.clone();
    assert!(run.respond(&cache, Response::Done).is_err());
    assert!(run.respond(&cache, Response::Resource(Some(0))).is_err());
    assert_eq!(run, before);
    cache.entries[0].guid = 12;
    assert!(run.advance(&cache).is_err());
    assert_eq!(run, before);
}
