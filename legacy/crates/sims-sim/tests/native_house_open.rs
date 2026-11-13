use serde_json::Value;
use sims_sim::lot_presenter::open::{Cursor, Request, State};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_f6_refresh_order_and_live_callback_state() {
    let report: Value = serde_json::from_str(include_str!("fixtures/house-open.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 320);
    fn patch(target: &mut Value, input: &Value) {
        if let Some(object) = input.as_object() {
            for (key, value) in object {
                patch(&mut target[key], value);
            }
        } else {
            *target = input.clone();
        }
    }
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut cursor = Cursor::begin(&state).unwrap();
        let mut events = vec![];
        while let Some(request) = cursor.request().cloned() {
            let bytes = serde_json::to_vec(&(&state, &cursor)).unwrap();
            (state, cursor) = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(serde_json::to_vec(&(&state, &cursor)).unwrap(), bytes);
            assert_eq!(cursor.returned(), None);
            let wrong = if request == Request::InvalidateView {
                Request::DrainIndicators
            } else {
                Request::InvalidateView
            };
            assert!(cursor.acknowledge(&mut state, &wrong).is_err());
            assert_eq!(serde_json::to_vec(&(&state, &cursor)).unwrap(), bytes);
            let index = events.len();
            events.push(serde_json::json!({"request":request,"state":state}));
            for change in c["changes"].as_array().unwrap() {
                if change["event"].as_u64().unwrap() as usize == index {
                    let mut value = serde_json::to_value(&state).unwrap();
                    patch(&mut value, &change["state"]);
                    state = serde_json::from_value(value).unwrap();
                }
            }
            cursor.acknowledge(&mut state, &request).unwrap();
            assert!(cursor.acknowledge(&mut state, &request).is_err());
        }
        assert!(cursor.complete());
        assert_eq!(
            serde_json::json!({"events":events,"state":state,"returned":cursor.returned()}),
            row["original"],
            "house-open case {n}"
        );
    }
}
