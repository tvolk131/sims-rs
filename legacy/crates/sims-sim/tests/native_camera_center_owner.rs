use serde_json::Value;
use sims_sim::lot_presenter::centering::{Cursor, Reply, Request, State};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_centering_floor_motion_order_and_live_callback_state() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/camera-center-owner.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 320);
    let mut saw_started = false;
    let mut saw_sample = false;
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut cursor = Cursor::begin(
            &state,
            serde_json::from_value(c["point"].clone()).unwrap(),
            c["target_level"].as_i64().unwrap() as i32,
            c["style"].as_u64().unwrap() as u8,
            c["immediate"].as_u64().unwrap() as u8,
        );
        let mut events = Vec::new();
        while let Some(request) = cursor.request() {
            (state, cursor) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, &cursor)).unwrap()).unwrap();
            assert_eq!(cursor.request(), Some(request.clone()));
            let wrong = match request {
                Request::Project { .. } | Request::SampleMotion { .. } => Reply::Done,
                _ => Reply::Screen([0, 0]),
            };
            let before = (state.clone(), cursor.clone());
            assert!(cursor.acknowledge(&mut state, &request, wrong).is_err());
            assert_eq!((&state, &cursor), (&before.0, &before.1));
            let index = events.len();
            events.push(serde_json::json!({"request":request,"state":state}));
            match request {
                Request::ChangeFloor { level, .. } => state.level = level,
                Request::ConfigureMotion { .. } => state.motion_active = 1,
                _ => (),
            }
            for change in c["changes"].as_array().unwrap() {
                if change["event"].as_u64().unwrap() as usize != index {
                    continue;
                }
                let mut patched = serde_json::to_value(&state).unwrap();
                for (key, value) in change["state"].as_object().unwrap() {
                    patched[key] = value.clone();
                }
                state = serde_json::from_value(patched).unwrap();
            }
            let reply = match request {
                Request::Project { .. } => {
                    Reply::Screen(serde_json::from_value(c["screen"].clone()).unwrap())
                }
                Request::SampleMotion { axis } => {
                    saw_sample = true;
                    Reply::Coordinate(c["sample"][axis as usize].as_i64().unwrap() as i32)
                }
                _ => Reply::Done,
            };
            cursor.acknowledge(&mut state, &request, reply).unwrap();
            assert!(
                cursor
                    .acknowledge(&mut state, &request, Reply::Done)
                    .is_err()
            );
        }
        assert!(cursor.complete());
        saw_started |=
            state.moving == 1 && state.application_value_bits == 0 && state.saved_value_bits != 99;
        assert_eq!(
            serde_json::json!({"events":events,"state":state}),
            row["original"],
            "center owner {n}"
        );
    }
    assert!(saw_started && saw_sample);
}
