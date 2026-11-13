use serde_json::{Value, json};
use sims_vm::interface::window_pause::{Change, Fields, Request};

fn run(input: &Value, resumed: bool) -> Value {
    let mut fields: Fields = serde_json::from_value(input["fields"].clone()).unwrap();
    let mut call = Change::new(
        input["paused"].as_u64().unwrap() as u8,
        input["overlay"].as_u64().unwrap() as u8,
    );
    let mut events = vec![];
    while let Some(request) = call.advance(&mut fields).unwrap() {
        if resumed {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            fields = serde_json::from_slice(&serde_json::to_vec(&fields).unwrap()).unwrap();
        }
        let before = fields.clone();
        assert_eq!(call.advance(&mut fields).unwrap(), Some(request.clone()));
        assert_eq!(fields, before);
        events.push(json!({"request":request,"fields":fields}));
        let kind = match request {
            Request::Unregister => "unregister",
            Request::Register(_) => "register",
            Request::Visible { .. } => "visible",
        };
        if input["patch"]["when"] == kind {
            let mut value = serde_json::to_value(&fields).unwrap();
            for (name, v) in input["patch"]["values"].as_object().unwrap() {
                value[name] = v.clone();
            }
            fields = serde_json::from_value(value).unwrap();
        }
        call.acknowledge(&request).unwrap();
        if resumed {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
    }
    assert!(call.complete());
    json!({"events":events,"fields":fields})
}

#[test]
fn pause_timer_replacement_precedes_flag_and_indicator_publication() {
    let input = json!({"fields":{"paused":0,"timer_active":255,"interval":80,"indicator":55},
        "paused":128,"overlay":1,"patch":{"when":"register","values":{"paused":0,"indicator":56}}});
    let result = run(&input, false);
    assert_eq!(result, run(&input, true));
    assert_eq!(
        result["events"],
        json!([
            {"request":"Unregister","fields":{"paused":0,"timer_active":255,"interval":80,"indicator":55}},
            {"request":{"Register":80},"fields":{"paused":128,"timer_active":0,"interval":80,"indicator":55}},
            {"request":{"Visible":{"indicator":56,"show":false}},"fields":{"paused":0,"timer_active":1,"interval":80,"indicator":56}}
        ])
    );
}

#[test]
fn pause_wrong_reply_preserves_the_pending_window_callback() {
    let mut fields = Fields {
        paused: 0,
        timer_active: 1,
        interval: 20,
        indicator: None,
    };
    let mut call = Change::new(0, 0);
    assert_eq!(
        call.advance(&mut fields).unwrap(),
        Some(Request::Unregister)
    );
    let before = call.clone();
    assert!(call.acknowledge(&Request::Register(20)).is_err());
    assert_eq!(call, before);
    call.acknowledge(&Request::Unregister).unwrap();
    assert_eq!(call.advance(&mut fields).unwrap(), None);
    assert_eq!(fields.timer_active, 0);
}

#[test]
#[ignore = "private original window pause corpus"]
fn window_pause_matches_original_callback_order_and_reentrant_fields() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/window-pause.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(corpus["native_execution"], true);
    let mut count = 0;
    for (n, case) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        if case["input"]["kind"] == "disabled_notice" {
            assert_eq!(
                case["original"],
                json!({"events":[],"diagnostic_pages_unchanged":true})
            );
        } else {
            assert_eq!(run(&case["input"], false), case["original"], "case {n}");
            assert_eq!(
                run(&case["input"], true),
                case["original"],
                "restored case {n}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 40);
}
