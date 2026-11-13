//! Original predicate/setter comparison; never starts an emulator from Cargo.
use serde_json::{Value, json};
use sims_sim::object_catalog::products::{Definition, Eligibility, Masks};
use sims_vm::interface::buy::{Fields, category::Change};

#[test]
#[ignore = "private original availability oracle; no emulator from Cargo"]
fn original_constructor_availability_matches_installed_products_and_combined_bits() {
    use sims_sim::object_catalog::products::categories::Availability;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/buy-availability-1.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 33);
    for c in cases {
        let mut availability = Availability::default();
        for p in c["input"]["products"].as_array().unwrap() {
            availability
                .add(serde_json::from_value(p.clone()).unwrap())
                .unwrap();
        }
        assert_eq!(
            serde_json::to_value(availability).unwrap(),
            c["original"],
            "{}",
            c["input"]["name"]
        );
    }
}

#[test]
#[ignore = "private original Buy oracle; run native_buy_catalog.py with no Cargo/rustc"]
fn original_buy_filters_and_category_page_setters_match_with_serialized_children() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/buy-catalog-1.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 820);
    for (n, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let answer = if c["kind"] == "predicates" {
            let bytes: Vec<u8> = serde_json::from_value(c["data"].clone()).unwrap();
            let d = Definition::parse(&bytes).unwrap();
            let masks = Masks {
                row: c["row"].as_u64().unwrap() as u32,
                column: c["column"].as_u64().unwrap() as u32,
            };
            let eligibility = &row["original"]["eligibility"];
            let expected = if eligibility[0] == true || eligibility[1] == true {
                Eligibility::Excluded
            } else if eligibility[2] == true {
                Eligibility::Included
            } else {
                Eligibility::Interactions
            };
            assert_eq!(d.eligibility(), expected, "case {n}");
            json!({"eligibility":eligibility,"present":(0..7).map(|c|d.present(c)).collect::<Vec<_>>(),
                "filters":(0..7).map(|c|d.matches(c,masks)).collect::<Vec<_>>()})
        } else {
            let mut fields: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
            let value = c["value"].as_i64().unwrap();
            let mut events = vec![];
            if c["kind"] == "category" {
                let mut change = Change::new(
                    c["setter"].as_u64().unwrap() as u8,
                    serde_json::from_value(c["axis"].clone()).unwrap(),
                    value as i32,
                )
                .unwrap();
                loop {
                    change = serde_json::from_slice(&serde_json::to_vec(&change).unwrap()).unwrap();
                    let Some(request) = change.advance(&mut fields).unwrap() else {
                        break;
                    };
                    events.push(json!({"request":request,"fields":fields}));
                    let mut patched = serde_json::to_value(&fields).unwrap();
                    for (k, v) in c["patch"].as_object().unwrap() {
                        patched[k] = v.clone();
                    }
                    fields = serde_json::from_value(patched).unwrap();
                    change.acknowledge(request).unwrap();
                }
                assert!(change.complete());
            } else if c["kind"] == "page" {
                fields.set_page(value as u32).unwrap();
            } else {
                fields.set_subcatalog(value as i32).unwrap();
            }
            json!({"events":events,"fields":fields})
        };
        assert_eq!(answer, row["original"], "original case {n}: {c}");
    }
}
