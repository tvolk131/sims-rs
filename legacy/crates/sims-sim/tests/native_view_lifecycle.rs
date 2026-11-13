use serde_json::Value;
use sims_sim::lot_presenter::{
    hover::Clear,
    lifecycle::{Cursor, Reply, Request, State},
};
fn event(events: &mut Vec<Value>, c: &Value, request: Value, state: &mut State) {
    let index = events.len();
    events.push(serde_json::json!({"request":request,"state":state}));
    for p in c["changes"].as_array().unwrap() {
        if p["event"].as_u64().unwrap() as usize != index {
            continue;
        }
        if let Some(v) = p["level"].as_i64() {
            state.level = v as i32;
        }
        for (name, field) in [
            ("all_walls", &mut state.all_walls),
            ("enabled", &mut state.visibility.enabled),
            ("dirty", &mut state.visibility.dirty),
            ("include_visible", &mut state.visibility.include_visible),
        ] {
            if let Some(v) = p[name].as_u64() {
                *field = v as u8;
            }
        }
        for row in p["rows"].as_array().into_iter().flatten() {
            state.visibility.mask.rows[row[0].as_u64().unwrap() as usize] =
                row[1].as_u64().unwrap();
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_mode_and_floor_callback_order_with_live_mutations() {
    let report: Value = serde_json::from_str(include_str!("fixtures/view-lifecycle.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 192);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut cursor = if let Some(v) = c["operation"]["Cutaway"].as_u64() {
            Cursor::cutaway(&mut state, v as u8).unwrap()
        } else {
            Cursor::floor(
                &state,
                c["operation"]["Floor"]["level"].as_i64().unwrap() as i32,
                c["operation"]["Floor"]["force"].as_u64().unwrap() as u8,
            )
            .unwrap()
        };
        let mut events = vec![];
        while let Some(request) = cursor.request() {
            (state, cursor) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, &cursor)).unwrap()).unwrap();
            cursor.validate(&state).unwrap();
            assert_eq!(cursor.request(), Some(request.clone()));
            let before = (state.clone(), cursor.clone());
            let wrong = if request == Request::ResolveSurface {
                Reply::Done
            } else {
                Reply::Surface(42)
            };
            assert!(cursor.acknowledge(&mut state, &request, wrong).is_err());
            assert_eq!((&state, &cursor), (&before.0, &before.1));
            event(
                &mut events,
                c,
                serde_json::to_value(&request).unwrap(),
                &mut state,
            );
            if request == Request::ClearMask {
                let mut clear = Clear::begin(
                    c["size"].as_u64().unwrap() as usize,
                    serde_json::from_value(c["origin"].clone()).unwrap(),
                )
                .unwrap();
                while let Some(p) = clear.advance(&mut state.visibility, state.level).unwrap() {
                    (state, cursor, clear) = serde_json::from_slice(
                        &serde_json::to_vec(&(&state, &cursor, &clear)).unwrap(),
                    )
                    .unwrap();
                    cursor.validate(&state).unwrap();
                    clear.validate().unwrap();
                    assert_eq!(
                        clear.advance(&mut state.visibility, state.level).unwrap(),
                        Some(p)
                    );
                    event(&mut events, c, serde_json::json!({"Tile":p}), &mut state);
                    clear.acknowledge(p).unwrap();
                }
                assert!(clear.complete());
            }
            let reply = if request == Request::ResolveSurface {
                Reply::Surface(7)
            } else {
                Reply::Done
            };
            cursor.acknowledge(&mut state, &request, reply).unwrap();
            assert!(
                cursor
                    .acknowledge(&mut state, &request, Reply::Done)
                    .is_err()
            );
        }
        cursor.validate(&state).unwrap();
        assert!(cursor.complete());
        assert_eq!(
            serde_json::json!({"events":events,"state":state}),
            row["original"],
            "native view lifecycle {n}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_refresh_active_and_encoded_floor_selection() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/room-mask-refresh.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 80);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let ids = sims_sim::lot_presenter::room_masks::refresh_ids(
            c["floor"].as_i64().unwrap() as i32,
            c["rooms"].as_array().unwrap().iter().map(|r| {
                (
                    r["id"].as_u64().unwrap() as u16,
                    r["active"].as_i64().unwrap() != 0,
                )
            }),
        );
        assert_eq!(serde_json::to_value(ids).unwrap(), row["original"]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn initialized_projection_fields_match_original_lookup_publication() {
    use sims_sim::lot_presenter::room_masks::Camera;
    let report: Value =
        serde_json::from_str(include_str!("fixtures/camera-floor-heights.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 24);
    for row in report["cases"].as_array().unwrap() {
        let origin = serde_json::from_value(row["input"]["origin"].clone()).unwrap();
        for camera in row["original"]["cameras"].as_array().unwrap() {
            let actual =
                Camera::initialized(origin, camera["zoom"].as_u64().unwrap() as u8).unwrap();
            assert_eq!(serde_json::to_value(actual).unwrap(), *camera);
        }
        // The probe's second pass must actually take the native cached path.
        let steps = row["original"]["steps"].as_array().unwrap();
        for reused in &steps[3..] {
            assert_eq!(reused["builders"], serde_json::json!([]));
            assert_eq!(reused["returned"], 0);
            assert_eq!(reused["state"], steps[2]["state"]);
        }
    }
    assert!(Camera::initialized([0, 0], 0).is_err());
    assert!(Camera::initialized([0, 0], 4).is_err());
}
