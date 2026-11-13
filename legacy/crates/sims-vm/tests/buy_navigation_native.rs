//! Read-only comparison of recorded native50633e executions; no emulator here.
use serde_json::{Value, json};
use sims_vm::interface::buy::{
    Fields,
    input::{Navigation, Request},
};

#[test]
#[ignore = "private original Buy navigation oracle; generate outside Cargo"]
fn original_buy_navigation_matches_all_catalogs_and_callback_mutations() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/buy-navigation-1.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 465);
    for (n, row) in cases.iter().enumerate() {
        let input = &row["input"];
        let mut fields: Fields = serde_json::from_value(input["fields"].clone()).unwrap();
        let initial_catalog = fields.catalog;
        let mut call =
            Navigation::new(serde_json::from_value(input["event"].clone()).unwrap()).unwrap();
        let mut events = vec![];
        for _ in 0..10 {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            let Some(request) = call.advance(&mut fields).unwrap() else {
                break;
            };
            events.push(json!({"request": request, "fields": fields}));
            if matches!(request, Request::Tool { .. }) && input["patch"] == true {
                fields.catalog = (initial_catalog + 1) % 7;
                fields.indices[initial_catalog as usize][1] = 5;
                fields.dirty = 0x80;
            }
            call.acknowledge(request).unwrap();
        }
        assert!(call.complete(), "case {n}");
        assert_eq!(
            json!({"fields": fields, "events": events}),
            row["original"],
            "case {n}: {input}"
        );
    }
}
