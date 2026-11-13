use serde_json::{Value, json};
use sims_sim::object_catalog::sprite_pool::Pool;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_sprite_pool_preserves_instance_identity_references_and_release_order() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/object-sprite-pool.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let mut pool = Pool::default();
        let mut events = vec![];
        for (i, op) in row["input"].as_array().unwrap().iter().enumerate() {
            let value = match op[0].as_str().unwrap() {
                "acquire" => Some(pool.acquire(op[1].as_u64().unwrap() as u32).unwrap()),
                "release" => {
                    let id = op[1].as_u64().unwrap() as u32;
                    if pool.release(id) {
                        events.push(json!(["destroy", id]));
                    }
                    None
                }
                "references" => {
                    pool.entries[op[1].as_u64().unwrap() as usize].references =
                        op[2].as_u64().unwrap() as u32;
                    None
                }
                _ => panic!("unknown pool operation"),
            };
            let entries=pool.entries.iter().map(|e|json!({"wrapper":e.wrapper,"resource":e.resource,"references":e.references})).collect::<Vec<_>>();
            assert_eq!(
                json!({"value":value,"entries":entries}),
                row["original"]["results"][i],
                "operation {i}: {op}"
            );
        }
        assert_eq!(json!(events), row["original"]["events"]);
    }
}
