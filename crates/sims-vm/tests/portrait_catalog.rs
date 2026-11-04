use serde::Deserialize;
use sims_vm::image_cache::{
    Entry, Event, State,
    catalog::{Host, Resource},
};

#[derive(Deserialize)]
struct Input {
    reset: bool,
    enabled: bool,
    entries: Vec<Entry>,
    refs: Vec<u32>,
    resources: Vec<Option<ResourceInput>>,
}
#[derive(Deserialize)]
struct ResourceInput {
    guid: u32,
    kind: u16,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Observation {
    Resource { index: u32, state: State },
    Release { image: u32, state: State },
    Destroy { image: u32, state: State },
}
#[derive(Deserialize)]
struct Output {
    state: State,
    events: Vec<Observation>,
}
#[derive(Deserialize)]
struct Row {
    input: Input,
    original: Output,
}
#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Row>,
}
struct Services<'a> {
    resources: &'a [Option<ResourceInput>],
    events: Vec<Observation>,
}
impl Host for Services<'_> {
    fn resource(&mut self, index: u32, cache: &State) -> Result<Option<Resource>, String> {
        self.events.push(Observation::Resource {
            index,
            state: cache.clone(),
        });
        Ok(self
            .resources
            .get(index as usize)
            .and_then(|r| r.as_ref())
            .map(|r| Resource {
                guid: r.guid,
                kind: r.kind,
            }))
    }
    fn image_event(&mut self, event: Event, cache: &State) {
        self.events.push(match event {
            Event::Release(image) => Observation::Release {
                image,
                state: cache.clone(),
            },
            Event::Destroy(image) => Observation::Destroy {
                image,
                state: cache.clone(),
            },
            Event::Ui => panic!("population does not invalidate UI"),
        });
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn cache_population_matches_native_order_duplicates_holes_and_ownership() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/portrait-catalog.json")).unwrap();
    assert_eq!(fixture.cases.len(), 96);
    for row in fixture.cases {
        let mut cache = State {
            entries: row.input.entries,
            refs: row.input.refs,
            ui: None,
        };
        let mut host = Services {
            resources: &row.input.resources,
            events: vec![],
        };
        cache
            .populate(row.input.reset, row.input.enabled, &mut host)
            .unwrap();
        assert_eq!(cache, row.original.state);
        assert_eq!(host.events, row.original.events);
        let restored: State = serde_json::from_slice(&serde_json::to_vec(&cache).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, cache);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn lazy_resource_resolution_matches_native_retries_and_cached_identity() {
    #[derive(Deserialize)]
    struct Input {
        responses: Vec<Option<u32>>,
    }
    #[derive(Deserialize)]
    struct Lookup {
        guid: u32,
        state: State,
    }
    #[derive(Deserialize)]
    struct Output {
        initial: State,
        state: State,
        events: Vec<Lookup>,
        returned: Vec<Option<u32>>,
    }
    #[derive(Deserialize)]
    struct Row {
        input: Input,
        original: Output,
    }
    #[derive(Deserialize)]
    struct Fixture {
        lookups: Vec<Row>,
    }
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/portrait-catalog.json")).unwrap();
    assert_eq!(fixture.lookups.len(), 18);
    for row in fixture.lookups {
        let mut cache = row.original.initial;
        let mut responses = row.input.responses.into_iter();
        let mut events = row.original.events.into_iter();
        for returned in row.original.returned {
            let result = cache
                .resolve_resource(0, |guid, cache| {
                    let event = events.next().unwrap();
                    assert_eq!(guid, event.guid);
                    assert_eq!(*cache, event.state);
                    Ok(responses.next().unwrap())
                })
                .unwrap();
            assert_eq!(result, returned);
            cache = serde_json::from_slice(&serde_json::to_vec(&cache).unwrap()).unwrap();
        }
        assert!(events.next().is_none());
        assert_eq!(cache, row.original.state);
    }
}
