use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::object_catalog::{self, Host, Key};

#[derive(Deserialize)]
struct Input {
    path: Vec<u8>,
    people: u8,
    category: u32,
    resource: u32,
    reacquired: u32,
    valid: u8,
    count: u32,
    ids: Vec<u32>,
    existing: Vec<[u32; 2]>,
    append: bool,
    repeat: usize,
}
struct Services<'a> {
    input: &'a Input,
    acquired: usize,
    catalog: Vec<Key<u32>>,
    events: Vec<Value>,
}
impl Host for Services<'_> {
    type Resource = u32;
    type Chunk = u32;
    fn acquire(&mut self, path: &[u8], people: u8) -> Result<Option<u32>, String> {
        let resource = if self.acquired == 0 {
            self.input.resource
        } else {
            self.input.reacquired
        };
        self.acquired += 1;
        self.events.push(json!(["acquire", path, people, resource]));
        Ok((resource != 0).then_some(resource))
    }
    fn valid(&mut self, resource: u32) -> Result<bool, String> {
        self.events
            .push(json!(["valid", resource, self.input.valid]));
        Ok(self.input.valid != 0)
    }
    fn count(&mut self, resource: u32) -> Result<u32, String> {
        self.events
            .push(json!(["count", resource, self.input.count]));
        Ok(self.input.count)
    }
    fn ordinal(&mut self, resource: u32, ordinal: u32) -> Result<u32, String> {
        self.events.push(json!(["ordinal", resource, ordinal]));
        Ok(ordinal)
    }
    fn id(&mut self, resource: u32, ordinal: u32) -> Result<u32, String> {
        let chunk = self.input.ids[ordinal as usize - 1];
        self.events.push(json!(["id", resource, ordinal, chunk]));
        Ok(chunk)
    }
    fn release_chunk(&mut self, resource: u32, chunk: u32) -> Result<(), String> {
        self.events.push(json!(["release_chunk", resource, chunk]));
        Ok(())
    }
    fn definitions(&self) -> &[Key<u32>] {
        &self.catalog
    }
    fn register(&mut self, resource: u32, id: u32, category: u32) -> Result<(), String> {
        self.events
            .push(json!(["register", resource, id, category]));
        if self.input.append {
            self.catalog.push(Key {
                resource,
                chunk: id as u16,
            });
        }
        Ok(())
    }
    fn release(&mut self, resource: u32) -> Result<(), String> {
        self.events.push(json!(["release", resource]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_iff_registration_preserves_identity_order_and_reference_handoffs() {
    #[derive(Deserialize)]
    struct Case {
        input: Input,
        original: Value,
    }
    #[derive(Deserialize)]
    struct Report {
        cases: Vec<Case>,
    }
    let report: Report =
        serde_json::from_str(include_str!("fixtures/object-registration.json")).unwrap();
    assert_eq!(report.cases.len(), 120);
    for (i, case) in report.cases.iter().enumerate() {
        let input = &case.input;
        let mut host = Services {
            input,
            acquired: 0,
            catalog: input
                .existing
                .iter()
                .map(|&[resource, chunk]| Key {
                    resource,
                    chunk: chunk as u16,
                })
                .collect(),
            events: vec![],
        };
        for _ in 0..input.repeat {
            host.acquired = 0;
            object_catalog::register_file(&input.path, input.category, input.people, &mut host)
                .unwrap();
        }
        let catalog: Vec<_> = host
            .catalog
            .iter()
            .map(|e| [e.resource, u32::from(e.chunk)])
            .collect();
        assert_eq!(
            json!({"events":host.events,"catalog":catalog}),
            case.original,
            "case {i}"
        );
    }
}
