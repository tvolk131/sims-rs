//!Recorded421986 comparison. Factory/removal/lookup/cast children were supplied
//!by the probe; their bodies are not proven by these outer-call traces.
use serde_json::{Value, json};
use sims_vm::{Object, World, cursor, multitile};
#[test]
#[ignore = "private original421986 corpus; generate only with no Cargo/rustc"]
fn original_purchase_wrapper_orders_children_and_marks_the_returned_cursor_tail() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/purchase-cursor-1.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 96);
    for (n, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let mut world = World::default();
        let mask = c["mask"].as_u64().unwrap();
        for i in 0..3 {
            let mut state = cursor::State::constructed(None, None);
            state.purchase = -10 - i;
            world.objects.insert(
                20 + i,
                Object {
                    cursor: (mask & (1 << i) != 0).then_some(state),
                    multitile: Some(multitile::Part {
                        head: (i != 0).then_some(20),
                        next: (i < 2).then_some(21 + i),
                        origin: [0; 3],
                        offset: [0; 3],
                    }),
                    ..Default::default()
                },
            );
        }
        let returned = c["returned"].as_i64().unwrap() as i16;
        let mut events = vec![];
        if c["definition"] == true {
            events.push(json!(["Furniture"]));
        }
        events.push(json!(["Cursor", c["furniture"], c["level"]]));
        if returned == 0 {
            events.push(json!(["Remove", c["furniture"]]));
        } else {
            events.push(json!(["Lookup", returned]));
            cursor::mark_purchase(&mut world, returned).unwrap();
        }
        let marks: Vec<_> = (0..3)
            .map(|i| {
                world.objects[&(20 + i)]
                    .cursor
                    .as_ref()
                    .map_or(-10 - i, |s| s.purchase)
            })
            .collect();
        assert_eq!(
            json!({"events":events,"returned":returned,"marks":marks}),
            row["original"],
            "case {n}: {c}"
        );
    }
}
