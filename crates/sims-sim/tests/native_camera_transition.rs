use serde_json::Value;
use sims_sim::lot_presenter::transition::{Cursor, Reply, Request, State};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_zoom_rotation_callbacks_centers_and_floor_variants() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/camera-transition.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 384);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut tiles: Vec<u16> = serde_json::from_value(c["tiles"].clone()).unwrap();
        let force = c["force"].as_u64().unwrap() as u8;
        let ui = c["refresh_ui"].as_u64().unwrap() as u8;
        let mut cursor = if c["operation"] == "zoom" {
            Cursor::zoom(&state, c["target"].as_u64().unwrap() as u8, force, ui)
        } else {
            Cursor::rotation(&state, c["target"].as_u64().unwrap() as u32, force, ui)
        }
        .unwrap();
        let mut events = vec![];
        while let Some(request) = cursor.request() {
            (state, tiles, cursor) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, &tiles, &cursor)).unwrap())
                    .unwrap();
            cursor.validate(&state).unwrap();
            assert_eq!(cursor.request(), Some(request.clone()));
            let wrong = match request {
                Request::MotionCoordinate { .. }
                | Request::ResolveCenter
                | Request::ResolveUi
                | Request::ResolveSurface => Reply::Done,
                _ => Reply::Coordinate(123),
            };
            let before = (state.clone(), tiles.clone(), cursor.clone());
            assert!(
                cursor
                    .acknowledge(&mut state, &mut tiles, &request, wrong)
                    .is_err()
            );
            assert_eq!((&state, &tiles, &cursor), (&before.0, &before.1, &before.2));
            let index = events.len();
            events.push(serde_json::json!({"request":request,"state":state}));
            if matches!(request, Request::RotateWorld { .. }) {
                state.world_rotation = c["world_rotation_reply"].as_u64().unwrap() as u32;
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
                for pair in change["tiles"].as_array().into_iter().flatten() {
                    tiles[pair[0].as_u64().unwrap() as usize] = pair[1].as_u64().unwrap() as u16;
                }
            }
            let reply = match request {
                Request::MotionCoordinate { axis } => {
                    Reply::Coordinate(c["motion"][axis as usize].as_i64().unwrap() as i32)
                }
                Request::ResolveCenter => {
                    Reply::Point(serde_json::from_value(c["center"].clone()).unwrap())
                }
                Request::ResolveUi => Reply::Token(c["ui_result"].as_bool().unwrap().then_some(33)),
                Request::ResolveSurface => Reply::Token(Some(7)),
                _ => Reply::Done,
            };
            cursor
                .acknowledge(&mut state, &mut tiles, &request, reply)
                .unwrap();
            assert!(
                cursor
                    .acknowledge(&mut state, &mut tiles, &request, Reply::Done)
                    .is_err()
            );
        }
        cursor.validate(&state).unwrap();
        assert!(cursor.complete());
        assert_eq!(
            serde_json::json!({"events":events,"state":state,"tiles":tiles}),
            row["original"],
            "transition {n}"
        );
    }
}
