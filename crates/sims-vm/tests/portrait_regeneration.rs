use serde::Deserialize;
use serde_json::{Value, json};
use sims_vm::image_cache::{
    Event, State,
    regeneration::{Regeneration, Request, Response},
};

#[derive(Deserialize)]
struct Row {
    input: Value,
    original: Output,
}
#[derive(Deserialize)]
struct Output {
    initial: State,
    state: State,
    events: Vec<Value>,
    returned: bool,
}
#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Row>,
}

fn image_event(event: Event, cache: &State) -> Value {
    match event {
        Event::Release(image) => json!({"kind":"release","image":image,"state":cache}),
        Event::Destroy(image) => json!({"kind":"destroy","image":image,"state":cache}),
        Event::Ui => panic!("regeneration does not directly invalidate UI"),
    }
}
fn optional(v: &Value) -> Option<u32> {
    v.as_u64().map(|n| n as u32)
}
fn flag(c: &Value, key: &str) -> bool {
    c[key].as_bool().unwrap()
}
fn start(c: &Value) -> Regeneration {
    Regeneration::new(
        c["index"].as_u64().unwrap() as u32,
        c["mask"].as_u64().unwrap() as u32,
        c["persist"].as_i64().unwrap() as i32,
        flag(c, "background").then_some(5),
        flag(c, "thumbnail").then_some(6),
        0x123456,
    )
}

#[cfg(feature = "private-fixtures")]
#[test]
fn regeneration_matches_native_masks_failures_lifetimes_and_resumes() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/portrait-regeneration.json")).unwrap();
    assert_eq!(fixture.cases.len(), 338);
    for (case, row) in fixture.cases.iter().enumerate() {
        for restore in [false, true] {
            let c = &row.input;
            let mut cache = row.original.initial.clone();
            let mut run = start(c);
            let mut events = vec![];
            loop {
                if restore {
                    let saved = serde_json::to_vec(&(&cache, &run)).unwrap();
                    (cache, run) = serde_json::from_slice(&saved).unwrap();
                }
                let Some(request) = run
                    .advance(&mut cache, &mut |event, cache| {
                        events.push(image_event(event, cache))
                    })
                    .unwrap()
                else {
                    break;
                };
                let suspended = serde_json::to_vec(&(&cache, &run)).unwrap();
                assert_eq!(
                    run.advance(&mut cache, &mut |_, _| panic!(
                        "duplicate poll mutated ownership"
                    ))
                    .unwrap(),
                    Some(request)
                );
                assert_eq!(suspended, serde_json::to_vec(&(&cache, &run)).unwrap());
                if restore {
                    (cache, run) = serde_json::from_slice(&suspended).unwrap();
                }
                let (mut event, response) = match request {
                    Request::Resource { guid } => (
                        json!({"kind":"resource","guid":guid}),
                        Response::Resource(flag(c, "resource").then_some(1)),
                    ),
                    Request::Find { index } => (
                        json!({"kind":"find","index":index}),
                        Response::Person((flag(c, "live") && c["guid"] != 0).then_some(11)),
                    ),
                    Request::Eligible { resource } => (
                        json!({"kind":"eligible","resource":resource}),
                        Response::Eligible(flag(c, "eligible")),
                    ),
                    Request::Create { index } => (
                        json!({"kind":"create","index":index}),
                        Response::Person(flag(c, "temporary").then_some(12)),
                    ),
                    Request::Clone { image } => (
                        json!({"kind":"clone","image":image}),
                        Response::Image {
                            image: optional(&c["clone"]),
                            success: true,
                        },
                    ),
                    Request::Render {
                        slot,
                        person,
                        input,
                    } => {
                        let result = &c["renders"][slot.to_string()];
                        (
                            json!({"kind":"render","slot":slot,"person":person,"input":input}),
                            Response::Image {
                                image: optional(&result["image"]),
                                success: flag(result, "success"),
                            },
                        )
                    }
                    Request::Key { image, color } => (
                        json!({"kind":"key","image":image,"color":color}),
                        Response::Done,
                    ),
                    Request::Compose { faces, background } => (
                        json!({"kind":"compose","faces":faces,"background":background}),
                        Response::Image {
                            image: optional(&c["composite"]),
                            success: flag(c, "compose_success"),
                        },
                    ),
                    Request::Persist {
                        index,
                        chunk,
                        image,
                    } => {
                        assert_eq!(index, 0);
                        (
                            json!({"kind":"persist","chunk":chunk,"image":image}),
                            Response::Done,
                        )
                    }
                    Request::Remove { person } => {
                        (json!({"kind":"remove","person":person}), Response::Done)
                    }
                    Request::Flush { resource } => {
                        (json!({"kind":"flush","resource":resource}), Response::Done)
                    }
                };
                event["state"] = serde_json::to_value(&cache).unwrap();
                events.push(event);
                run.respond(&mut cache, response, &mut |event, cache| {
                    events.push(image_event(event, cache))
                })
                .unwrap();
                run.validate(&cache).unwrap();
            }
            assert_eq!(
                run.returned(),
                Some(row.original.returned),
                "return case {case}"
            );
            assert_eq!(cache, row.original.state, "state case {case}");
            // Optional cache metadata is omitted from Rust snapshots when
            // empty; compare the decoded state, not JSON null versus absence.
            let mut expected = row.original.events.clone();
            for event in &mut expected {
                let state: State = serde_json::from_value(event["state"].clone()).unwrap();
                event["state"] = serde_json::to_value(state).unwrap();
            }
            assert_eq!(events, expected, "events case {case}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn regeneration_rejects_mismatched_responses_and_lost_owners() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/portrait-regeneration.json")).unwrap();
    let row = fixture
        .cases
        .iter()
        .find(|r| r.input["mask"] == 31 && r.input["live"] == true)
        .unwrap();
    let mut cache = row.original.initial.clone();
    let mut run = start(&row.input);
    let request = run.advance(&mut cache, &mut |_, _| {}).unwrap();
    assert!(matches!(request, Some(Request::Resource { .. })));
    let before = (cache.clone(), run.clone());
    assert!(
        run.respond(&mut cache, Response::Done, &mut |_, _| panic!())
            .is_err()
    );
    assert_eq!((cache.clone(), run.clone()), before);
    assert!(
        run.respond(
            &mut cache,
            Response::Resource(Some(0)),
            &mut |_, _| panic!()
        )
        .is_err()
    );
    assert_eq!((cache.clone(), run.clone()), before);
    cache.refs[4] = 0; // global background owner must survive suspended work
    assert!(run.validate(&cache).is_err());
    let (mut cache, mut run) = before;
    run.respond(&mut cache, Response::Resource(Some(1)), &mut |_, _| {})
        .unwrap();
    assert!(matches!(
        run.advance(&mut cache, &mut |_, _| {}).unwrap(),
        Some(Request::Find { .. })
    ));
    run.respond(&mut cache, Response::Person(Some(11)), &mut |_, _| {})
        .unwrap();
    assert!(matches!(
        run.advance(&mut cache, &mut |_, _| {}).unwrap(),
        Some(Request::Render { slot: 0, .. })
    ));
    run.respond(
        &mut cache,
        Response::Image {
            image: Some(7),
            success: true,
        },
        &mut |_, _| {},
    )
    .unwrap();
    let mut malformed = serde_json::to_value(&run).unwrap();
    malformed["cursor"] = json!(6);
    let malformed: Regeneration = serde_json::from_value(malformed).unwrap();
    assert!(malformed.validate(&cache).is_err());
    cache.refs[6] = 0; // the suspended render owns its output wrapper
    assert!(run.validate(&cache).is_err());
}
