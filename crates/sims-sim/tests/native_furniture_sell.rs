//! Original487e18 control/arithmetic cases, not a complete sale integration.
use serde_json::{Value, json};
use sims_sim::{
    edit_history::object::budget,
    object_catalog::move_tool::{cleanup, sell},
};
use std::collections::BTreeMap;

struct View {
    underlying: bool,
    costs: BTreeMap<u32, i32>,
}
impl sell::View for View {
    fn underlying(&self, cursor: u32) -> Result<Option<u32>, String> {
        if !self.underlying {
            return Ok(None);
        }
        Ok(Some(match cursor {
            10 => 20,
            11 => 21,
            _ => return Err("unknown cursor".into()),
        }))
    }
    fn set_payload_cost(&mut self, payload: u32, cost: i32) -> Result<(), String> {
        *self.costs.get_mut(&payload).ok_or("unknown command")? = cost;
        Ok(())
    }
}
fn number(v: &Value, key: &str) -> i32 {
    v[key].as_i64().unwrap() as i32
}
fn flag(v: &Value, key: &str) -> u8 {
    v[key].as_u64().unwrap() as u8
}
fn mutate(input: &Value, when: &str, tool: &mut cleanup::State, budget: &mut budget::State) {
    match (when, input["mutation"].as_str().unwrap()) {
        ("permission", "replace_held") => tool.held = Some(11),
        ("permission", "clear_held") => tool.held = None,
        ("detach", "preview_after_detach") => tool.tool.flags[0] = 1,
        ("price", "replace_payload") => tool.payload = Some(31),
        ("price", "disable_refund") => tool.tool.flags[3] = 0,
        ("budget", "pending_in_budget") => budget.pending = 123456,
        ("reset", "clear_in_reset") | ("deactivate", "clear_in_deactivate") => {
            tool.held = None;
            tool.payload = None;
        }
        _ => (),
    }
}
fn run(input: &Value, resume: bool) -> Value {
    let mut tool = cleanup::State::constructor_fields(40);
    tool.held = input["held"].as_bool().unwrap().then_some(10);
    tool.payload = input["payload"].as_bool().unwrap().then_some(30);
    tool.tool.flags[0] = flag(input, "preview");
    tool.tool.flags[2] = flag(input, "discount");
    tool.tool.flags[3] = flag(input, "refund");
    let mut budget = budget::State {
        pending: number(input, "pending"),
        category: number(input, "category"),
    };
    let mut view = View {
        underlying: input["underlying"].as_bool().unwrap(),
        costs: [(30, -7), (31, -8)].into(),
    };
    let mut call = sell::Call::default();
    let mut events = vec![];
    for _ in 0..32 {
        let Some(request) = call.advance(&mut tool, &mut budget, &mut view).unwrap() else {
            break;
        };
        let before = (tool.clone(), budget, view.costs.clone(), call.clone());
        assert_eq!(
            call.advance(&mut tool, &mut budget, &mut view).unwrap(),
            Some(request)
        );
        assert_eq!(
            (tool.clone(), budget, view.costs.clone(), call.clone()),
            before,
            "pending call replay mutated state"
        );
        if resume {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
        use sell::{Feedback as F, Reply as R, Request as Q};
        let reply = match request {
            Q::CanSell(object) => {
                events.push(json!(["CanSell", object]));
                mutate(input, "permission", &mut tool, &mut budget);
                R::Bool(flag(input, "can_sell") != 0)
            }
            Q::Placed(cursor) => {
                events.push(json!(["Placed", cursor]));
                R::Bool(input["placed"].as_bool().unwrap())
            }
            Q::Detach(cursor) => {
                events.push(json!(["Detach", cursor]));
                mutate(input, "detach", &mut tool, &mut budget);
                R::Done
            }
            Q::Resale(object) => {
                events.push(json!(["Resale", object]));
                mutate(input, "price", &mut tool, &mut budget);
                R::Price(number(input, "price") as i16)
            }
            Q::Budget { category, amount } => {
                events.push(json!(["Budget", category, amount]));
                mutate(input, "budget", &mut tool, &mut budget);
                R::Done
            }
            Q::Feedback(feedback) => {
                events.push(json!([
                    "Feedback",
                    match feedback {
                        F::Blocked => "Blocked",
                        F::Delete => "Delete",
                        F::MoneyBack => "MoneyBack",
                    }
                ]));
                R::Done
            }
            Q::Reset => {
                events.push(json!(["Reset"]));
                mutate(input, "reset", &mut tool, &mut budget);
                R::Done
            }
            Q::Deactivate => {
                events.push(json!(["Deactivate"]));
                mutate(input, "deactivate", &mut tool, &mut budget);
                R::Done
            }
        };
        call.acknowledge(request, reply).unwrap();
    }
    assert!(call.complete());
    json!({"events":events,"pending":budget.pending,"category":budget.category,
        "held":tool.held,"payload":tool.payload,"flags":[tool.tool.flags[0],tool.tool.flags[2],tool.tool.flags[3]],
        "costs":[view.costs[&30],view.costs[&31]]})
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_sell_matches_original_order_arithmetic_and_each_suspended_reply() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-furniture-sell.json")).unwrap();
    assert_eq!(
        fixture["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        fixture["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 468);
    for (index, case) in cases.iter().enumerate() {
        for resume in [false, true] {
            assert_eq!(
                run(&case["input"], resume),
                case["original"],
                "case{index}, resumed{resume}"
            );
        }
    }
}

#[test]
fn sell_rejects_foreign_and_wrong_reply_without_consuming_the_request() {
    let mut tool = cleanup::State::constructor_fields(40);
    tool.held = Some(10);
    let mut budget = budget::State {
        pending: 0,
        category: 7,
    };
    let mut view = View {
        underlying: true,
        costs: BTreeMap::new(),
    };
    let mut call = sell::Call::default();
    let request = call
        .advance(&mut tool, &mut budget, &mut view)
        .unwrap()
        .unwrap();
    let before = call.clone();
    assert!(
        call.acknowledge(sell::Request::CanSell(21), sell::Reply::Bool(true))
            .is_err()
    );
    assert_eq!(call, before);
    assert!(call.acknowledge(request, sell::Reply::Done).is_err());
    assert_eq!(call, before);
    call.acknowledge(request, sell::Reply::Bool(false)).unwrap();
    assert!(call.acknowledge(request, sell::Reply::Bool(false)).is_err());
}
