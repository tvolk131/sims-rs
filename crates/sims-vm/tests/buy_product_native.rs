//! Recorded native comparison; this test never imports or starts an emulator.
use serde_json::{Value, json};
use sims_vm::interface::buy::{
    Fields,
    product::{Click, Reply, Request, Slot},
};
#[test]
#[ignore = "private original5065c8 probe; generate with no Cargo/rustc"]
fn original_product_click_matches_live_page_cost_capture_and_feedback() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/buy-product-1.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 301);
    for (case, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let mut fields: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
        let clicked = c["clicked"].as_u64().unwrap() as u32;
        let mut call = Click::new(clicked);
        let mut items: Vec<Option<u32>> = serde_json::from_value(c["items"].clone()).unwrap();
        let mut retained = items.clone();
        let widgets: Vec<bool> = serde_json::from_value(c["widgets"].clone()).unwrap();
        let mut prices: Vec<i32> = serde_json::from_value(c["prices"].clone()).unwrap();
        let mut funds = c["funds"].as_i64().unwrap() as i32;
        let mut details = c["details"].as_bool().unwrap();
        let now = c["now"].as_u64().unwrap() as u32;
        let mut last = c["last_error"].as_u64().unwrap() as u32;
        let mut events = vec![];
        for _ in 0..100 {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            let Some(r) = call.advance(&mut fields).unwrap() else {
                break;
            };
            let reply = match r {
                Request::Read(i) => Reply::Slot(match items.get(i as usize) {
                    None => Slot::End,
                    Some(None) => Slot::Empty,
                    Some(Some(id)) => Slot::Item {
                        product: *id,
                        widget: widgets[*id as usize - 1].then_some(*id),
                    },
                }),
                Request::Cost(id) => Reply::Cost {
                    funds: if c["community"] == true {
                        999999
                    } else {
                        funds
                    },
                    price: prices[id as usize - 1],
                },
                Request::BuyTool => {
                    events.push(json!({"kind":"Tool","fields":fields}));
                    if fields.selected_object != 0 {
                        match c["patch"].as_str() {
                            Some("funds") => {
                                funds = 10000;
                                prices[clicked as usize - 1] = 0;
                            }
                            Some("replace") => {
                                items = vec![Some(3), Some(1), Some(2)];
                                retained = items.clone();
                            }
                            Some("shrink") => items.clear(),
                            Some("details") => details = c["details"] != true,
                            None => (),
                            _ => panic!(),
                        }
                    }
                    Reply::Done
                }
                Request::DetailsIfOpen(i) => {
                    if details {
                        events.push(json!({"kind":"Details","product":retained[i as usize].unwrap_or(0),"fields":fields}));
                    }
                    Reply::Done
                }
                Request::Widget { widget, selected } => {
                    events.push(json!({"kind":"Widget","widget":widget,"selected":selected,"fields":fields}));
                    Reply::Done
                }
                Request::ErrorFeedback => {
                    events.push(json!({"kind":"Error","fields":fields}));
                    if last.wrapping_add(400) < now {
                        last = now;
                    }
                    Reply::Done
                }
            };
            call.acknowledge(r, reply).unwrap();
        }
        assert!(call.complete(), "case {case}");
        assert_eq!(
            json!({"fields":fields,"events":events,"last_error":last}),
            row["original"],
            "case {case}: {c}"
        );
    }
}
