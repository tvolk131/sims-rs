#![cfg(feature = "private-fixtures")]

use serde::Deserialize;
use serde_json::{Value, json};
use sims_vm::{
    dialog::{IconLookup, resource_icon},
    image_cache::{
        Event, State,
        portrait::{Portrait, Request, Response},
    },
};

#[derive(Deserialize)]
struct Fixture {
    selectors: Vec<SelectorRow>,
    portraits: Vec<PortraitRow>,
}
#[derive(Deserialize)]
struct SelectorRow {
    input: SelectorInput,
    original: Option<u32>,
}
#[derive(Debug, Deserialize)]
struct SelectorInput {
    flags: u8,
    stack: i16,
    formatted: bool,
    caller: bool,
    neighbor_resource: bool,
}
#[derive(Deserialize)]
struct PortraitRow {
    input: Input,
    original: Output,
}
#[derive(Debug, Deserialize)]
struct Input {
    index: i32,
    background: bool,
    out: Option<u32>,
    refresh: u32,
    persist: u32,
    saved: Option<u32>,
    composite: Option<u32>,
}
#[derive(Deserialize)]
struct Output {
    initial: State,
    state: State,
    returned: bool,
    out: Option<u32>,
    events: Vec<Value>,
    destroyed: Vec<u32>,
    releases: Vec<Value>,
}
fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/dialog-portrait.json")).unwrap()
}

#[test]
fn resource_selection_matches_original_with_distinct_namespaces() {
    let rows = fixture().selectors;
    assert_eq!(rows.len(), 768);
    for row in rows {
        let c = row.input;
        let selected = resource_icon(
            c.flags,
            c.formatted.then_some(5),
            c.caller,
            c.stack,
            |key| match key {
                IconLookup::Live(1) => Some(1),
                IconLookup::Live(3) => Some(2),
                IconLookup::Neighbor(1) if c.neighbor_resource => Some(3),
                IconLookup::Neighbor(3) if c.neighbor_resource => Some(4),
                _ => None,
            },
        )
        .unwrap();
        assert_eq!(selected, row.original, "{c:?}");
    }
}

#[test]
fn portrait_cache_matches_original_and_restores_every_boundary() {
    let rows = fixture().portraits;
    assert_eq!(rows.len(), 216);
    for row in rows {
        for restore in [false, true] {
            let c = &row.input;
            let mut cache = row.original.initial.clone();
            let mut portrait = Portrait::new(c.index as u32, c.out, c.background.then_some(5));
            let mut events = vec![];
            let mut destroyed = vec![];
            let mut releases = vec![];
            let mut observer = |event, state: &State| match event {
                Event::Destroy(image) => destroyed.push(image),
                Event::Release(image) => releases.push(json!({"image":image,"state":state})),
                Event::Ui => panic!("portrait getter does not invalidate UI"),
            };
            loop {
                if restore {
                    (cache, portrait) =
                        serde_json::from_slice(&serde_json::to_vec(&(&cache, &portrait)).unwrap())
                            .unwrap();
                }
                portrait.validate(&cache).unwrap();
                let Some(request) = portrait.advance(&mut cache, &mut observer).unwrap() else {
                    break;
                };
                // Re-polling and rejecting the wrong response cannot execute a
                // service twice or alter either owner's saved state.
                let saved = (cache.clone(), portrait.clone());
                assert_eq!(
                    portrait.advance(&mut cache, &mut observer).unwrap(),
                    Some(request)
                );
                let wrong = if matches!(request, Request::Refresh { .. }) {
                    Response::Image(None)
                } else {
                    Response::Refreshed
                };
                assert!(portrait.respond(&mut cache, wrong, &mut observer).is_err());
                assert_eq!((&cache, &portrait), (&saved.0, &saved.1));
                if restore {
                    (cache, portrait) =
                        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
                }
                let response = match request {
                    Request::Refresh { index, persist } => {
                        assert_eq!(index, c.index as u32);
                        events.push(json!({"kind":"refresh", "persist":persist, "state":cache}));
                        let result = if persist { c.persist } else { c.refresh };
                        // Same supplied service effects as the native harness;
                        // regeneration itself is outside this comparison.
                        if result != 0 {
                            cache
                                .store_image(index as usize, 0, Some(3), &mut |_, _| {})
                                .unwrap();
                        }
                        if result == 1 {
                            cache
                                .store_image(index as usize, 1, Some(4), &mut |_, _| {})
                                .unwrap();
                        }
                        Response::Refreshed
                    }
                    Request::Load { index, chunk } => {
                        assert_eq!((index, chunk), (c.index as u32, 2002));
                        events.push(json!({"kind":"load", "state":cache}));
                        Response::Image(c.saved)
                    }
                    Request::Compose { faces, background } => {
                        assert_eq!((Some(faces), background), (c.saved, 5));
                        events.push(json!({"kind":"compose", "state":cache}));
                        Response::Image(c.composite)
                    }
                };
                portrait
                    .respond(&mut cache, response, &mut observer)
                    .unwrap();
            }
            portrait.validate(&cache).unwrap();
            assert_eq!(
                portrait.finish().unwrap(),
                (row.original.returned, row.original.out),
                "{c:?}"
            );
            assert_eq!(cache, row.original.state, "{c:?}");
            assert_eq!(events, row.original.events, "{c:?}");
            assert_eq!(destroyed, row.original.destroyed, "{c:?}");
            assert_eq!(releases, row.original.releases, "{c:?}");
        }
    }
}

#[test]
fn portrait_rejects_lost_image_ownership_and_malformed_responses_atomically() {
    let mut cache = fixture().portraits[0].original.initial.clone();
    let mut portrait = Portrait::new(0, None, Some(5));
    assert_eq!(
        portrait.advance(&mut cache, &mut |_, _| {}).unwrap(),
        Some(Request::Load {
            index: 0,
            chunk: 2002
        })
    );
    for image in [0, 7, u32::MAX] {
        let before = (cache.clone(), portrait.clone());
        assert!(
            portrait
                .respond(&mut cache, Response::Image(Some(image)), &mut |_, _| {})
                .is_err()
        );
        assert_eq!((&cache, &portrait), (&before.0, &before.1));
    }
    portrait
        .respond(&mut cache, Response::Image(Some(2)), &mut |_, _| {})
        .unwrap();
    assert_eq!(
        portrait.advance(&mut cache, &mut |_, _| {}).unwrap(),
        Some(Request::Compose {
            faces: 2,
            background: 5
        })
    );
    // Serialized cache ownership can be valid by itself while the suspended
    // portrait's local owner is missing. The joint check must catch that.
    cache.refs[1] = 1;
    assert!(cache.validate().is_ok());
    let before = (cache.clone(), portrait.clone());
    assert!(portrait.advance(&mut cache, &mut |_, _| {}).is_err());
    assert!(
        portrait
            .respond(&mut cache, Response::Image(None), &mut |_, _| {})
            .is_err()
    );
    assert_eq!((&cache, &portrait), (&before.0, &before.1));
}
