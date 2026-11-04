use serde_json::Value;
use sims_sim::lot_presenter::hover::{Geometry, History, Mouse, Request, View};
fn compare(report: Value, count: usize) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), count);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut view: View = serde_json::from_value(c["view"].clone()).unwrap();
        let mut history = History {
            capacity: view.history.capacity,
            rooms: vec![],
        };
        let mut steps = vec![serde_json::to_value(&history).unwrap()];
        for op in c["history_ops"].as_array().unwrap() {
            if let Some(v) = op.as_u64() {
                history.insert(v as u32).unwrap();
            } else {
                history.clear();
            }
            steps.push(serde_json::to_value(&history).unwrap());
        }
        assert_eq!(
            serde_json::to_value(steps).unwrap(),
            expected["history_steps"]
        );
        assert_eq!(history, view.history);
        let primary: Vec<i16> = serde_json::from_value(c["primary"].clone()).unwrap();
        let geometry = Geometry {
            size: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
            primary: &primary,
        };
        let mut mouse = Mouse::begin(
            &mut view,
            serde_json::from_value(c["screen"].clone()).unwrap(),
            c["flags"].as_u64().unwrap() as u32,
            c["base_reply"].as_u64().unwrap() as u8,
        )
        .unwrap();
        let mut requests = vec![];
        let mut effect = Value::Null;
        while let Some(request) = mouse.request().cloned() {
            let bytes = serde_json::to_vec(&(&view, &mouse)).unwrap();
            (view, mouse) = serde_json::from_slice(&bytes).unwrap();
            mouse.validate(&view).unwrap();
            assert_eq!(mouse.request(), Some(&request));
            match request {
                Request::Project { screen, flag } => {
                    assert_eq!(flag, 1);
                    let points = c["points"].as_array().unwrap();
                    let point = serde_json::from_value(
                        points[requests.len().min(points.len() - 1)].clone(),
                    )
                    .unwrap();
                    let wrong = [screen[0], screen[1].wrapping_add(1)];
                    assert!(mouse.projected(&mut view, &geometry, wrong, point).is_err());
                    requests.push(serde_json::json!({"screen":screen,"flag":flag}));
                    mouse
                        .projected(&mut view, &geometry, screen, point)
                        .unwrap();
                }
                Request::ComposeVisible | Request::ClearMask => {
                    effect = Value::String(
                        if request == Request::ComposeVisible {
                            "compose"
                        } else {
                            "clear"
                        }
                        .into(),
                    );
                    mouse.acknowledge(&view, &request).unwrap();
                    assert!(mouse.acknowledge(&view, &request).is_err());
                }
            }
        }
        assert!(mouse.complete());
        mouse.validate(&view).unwrap();
        assert_eq!(
            mouse.returned(),
            Some(expected["returned"].as_u64().unwrap() as u8)
        );
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            expected["view"],
            "native mouse fields {n}"
        );
        assert_eq!(
            serde_json::to_value(requests).unwrap(),
            expected["requests"],
            "native projection order {n}"
        );
        assert_eq!(effect, expected["effect"], "native mouse effect {n}");
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_mouse_room_history_and_projection_requests() {
    compare(
        serde_json::from_str(include_str!("fixtures/hover-rooms.json")).unwrap(),
        128,
    );
}
#[test]
#[ignore = "requires private original-house mouse room evidence"]
fn original_house_mouse_room_queries() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-hover-rooms.json");
    compare(
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap(),
        24,
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_clear_live_notifications_and_resume() {
    use sims_sim::lot_presenter::{hover::Clear, visibility::State};
    let report: Value = serde_json::from_str(include_str!("fixtures/cutaway-clear.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 64);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut level = c["level"].as_i64().unwrap() as i32;
        let mut clear = Clear::begin(
            c["size"].as_u64().unwrap() as usize,
            serde_json::from_value(c["origin"].clone()).unwrap(),
        )
        .unwrap();
        let mut events = vec![];
        while let Some(p) = clear.advance(&mut state, level).unwrap() {
            assert_eq!(clear.advance(&mut state, level).unwrap(), Some(p));
            (state, clear) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, &clear)).unwrap()).unwrap();
            clear.validate().unwrap();
            assert_eq!(clear.advance(&mut state, level).unwrap(), Some(p));
            let mut wrong = p;
            wrong[2] = 0;
            assert!(clear.acknowledge(wrong).is_err());
            for v in c["changes"].as_array().unwrap() {
                if v["after_event"].as_u64().unwrap() as usize == events.len() {
                    state.mask.rows[v["row"].as_u64().unwrap() as usize] =
                        v["bits"].as_u64().unwrap();
                    level = v["level"].as_i64().unwrap() as i32;
                }
            }
            events.push(p);
            clear.acknowledge(p).unwrap();
            assert!(clear.acknowledge(p).is_err());
        }
        assert!(clear.complete());
        clear.validate().unwrap();
        assert_eq!(
            serde_json::json!({"state":state,"events":events,"level":level}),
            row["original"],
            "native clear {n}"
        );
    }
}
