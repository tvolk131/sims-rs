use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_sim::object_catalog::resources::{Cache, Host};

struct Resource {
    filename: Vec<u8>,
    valid: u8,
}
struct Services<'a> {
    operation: &'a Value,
    resources: Vec<Resource>,
    events: Vec<Value>,
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::from_value(value.clone()).unwrap()
}
impl Host for Services<'_> {
    type Resource = usize;
    fn construct(&mut self) -> Result<usize, String> {
        self.resources.push(Resource {
            filename: bytes(&self.operation["filename"]),
            valid: self.operation["valid"].as_u64().unwrap() as u8,
        });
        let id = self.resources.len();
        self.events.push(json!(["construct", id]));
        Ok(id)
    }
    fn open(&mut self, id: usize, path: &[u8], mode: u8) -> Result<(), String> {
        self.events.push(json!(["open", id, path, mode]));
        Ok(())
    }
    fn valid(&mut self, id: usize) -> Result<bool, String> {
        let value = self.resources[id - 1].valid;
        self.events.push(json!(["valid", id, value]));
        Ok(value != 0)
    }
    fn filename(&mut self, id: usize) -> Result<Vec<u8>, String> {
        let value = self.resources[id - 1].filename.clone();
        self.events.push(json!(["filename", id, value]));
        Ok(value)
    }
    fn close(&mut self, id: usize) -> Result<(), String> {
        self.events.push(json!(["close", id]));
        Ok(())
    }
    fn destroy(&mut self, id: usize) -> Result<(), String> {
        self.events.push(json!(["destroy", id]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_resource_cache_preserves_keys_rehash_counts_and_lifetime() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/object-resource-cache.json")).unwrap();
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 46);
    let mut operations = 0;
    for (i, case) in cases.iter().enumerate() {
        let mut cache = Cache::default();
        let mut host = Services {
            operation: &Value::Null,
            resources: vec![],
            events: vec![],
        };
        let mut results = vec![];
        let entries = |cache: &Cache<usize>| -> Vec<Value> {
            cache
                .entries()
                .map(|e| json!({"key":e.key,"references":e.references,"resource":e.resource}))
                .collect()
        };
        for (step, operation) in case["input"].as_array().unwrap().iter().enumerate() {
            operations += 1;
            host.operation = operation;
            let value = match operation["kind"].as_str().unwrap() {
                "acquire" => json!(
                    cache
                        .acquire(
                            &bytes(&operation["path"]),
                            operation["people"].as_u64().unwrap() as u8,
                            &mut host
                        )
                        .unwrap()
                        .unwrap_or(0)
                ),
                "valid" => {
                    host.resources[operation["resource"].as_u64().unwrap() as usize - 1].valid =
                        operation["valid"].as_u64().unwrap() as u8;
                    Value::Null
                }
                "release" => {
                    cache
                        .release(operation["resource"].as_u64().unwrap() as usize, &mut host)
                        .unwrap();
                    Value::Null
                }
                _ => unreachable!(),
            };
            let state = entries(&cache);
            let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&state).unwrap()));
            let result = json!({"result":value,"count":state.len(),"buckets":cache.bucket_count(),"ordered_state_sha256":digest});
            assert_eq!(
                result, case["original"]["results"][step],
                "case {i} step {step}: {state:?}"
            );
            results.push(result);
        }
        assert_eq!(
            json!({"events":host.events,"results":results,"final_entries":entries(&cache)}),
            case["original"],
            "case {i}"
        );
    }
    assert_eq!(operations, 2971);
}
