use serde_json::{Value, json};
use sims_sim::pose::{
    movement::{Movement, Request, route::Poll},
    ordinary,
    transform::Placement,
};

#[cfg(feature = "private-fixtures")]
#[test]
fn pending_stop_continuations_match_original_route_wrapper() {
    use sims_sim::pose::movement::route::Stop;
    let report: Value =
        serde_json::from_str(include_str!("fixtures/route-walk-continuation.json")).unwrap();
    let mut count = 0;
    for row in report["cases"].as_array().unwrap() {
        let stop = match row["input"]["state"].as_u64().unwrap() {
            9 => Stop::Success,
            10 => Stop::Retry,
            11 => Stop::Failure,
            _ => continue,
        };
        assert_eq!(
            json!(stop.frame_continuation()),
            row["original"]["continuation"]
        );
        assert_eq!(row["original"]["returned"], 2);
        assert_eq!(row["original"]["events"], json!(["stop_route_pending"]));
        count += 1;
    }
    assert_eq!(count, 9);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_route_start_poll_stop_preserve_layers_and_retained_braking_state() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-movement-route-control.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 132);
    for row in rows {
        let before = &row["before"];
        let expected = &row["original"];
        let input = &row["input"];
        let mut movement = Movement::new(serde_json::from_value(before["state"].clone()).unwrap());
        movement.path = Some(serde_json::from_value(before["path"].clone()).unwrap());
        movement.path.as_mut().unwrap().stop_mode = before["stop_mode"].as_i64().unwrap() as i32;
        movement.scalar_7c = before["scalar_7c"].as_f64().unwrap() as f32;
        let placement: Placement = serde_json::from_value(before["placement"].clone()).unwrap();
        let control: ordinary::State = serde_json::from_value(before["ordinary"].clone()).unwrap();
        let mut object22 = before["object22"].as_i64().unwrap() as i16;
        let mut facing = before["facing"].as_i64().unwrap() as i16;
        let (mut movement, mut placement, mut control): (Movement, Placement, ordinary::State) =
            serde_json::from_slice(&serde_json::to_vec(&(movement, placement, control)).unwrap())
                .unwrap();
        if input["operation"] == "start" {
            object22 = movement
                .start_route(
                    &mut placement,
                    &mut control,
                    serde_json::from_value(input["position"].clone()).unwrap(),
                    input["facing"].as_i64().unwrap() as i16,
                )
                .unwrap();
            assert_eq!(row["trace"], json!([[0x6f6d64, 6], [0x6f6d68, 6]]));
        } else if input["operation"] == "poll" {
            let mut poll = Poll::begin(
                &movement,
                &mut placement,
                &mut facing,
                serde_json::from_value(input["position"].clone()).unwrap(),
                input["level"].as_i64().unwrap() as i32,
            )
            .unwrap();
            for event in row["events"].as_array().unwrap() {
                let request = poll.request().unwrap();
                assert_eq!(json!(request), event["request"]);
                assert_eq!(
                    placement,
                    serde_json::from_value(event["snapshot"]["placement"].clone()).unwrap()
                );
                assert_eq!(json!(facing), event["snapshot"]["facing"]);
                let restored: Poll =
                    serde_json::from_slice(&serde_json::to_vec(&poll).unwrap()).unwrap();
                assert_eq!(poll, restored);
                poll = restored;
                match request {
                    Request::AcceptStep { .. } => poll
                        .accept(&mut placement, input["accepted"] == true)
                        .unwrap(),
                    Request::MoveStep { .. } => poll.moved().unwrap(),
                    _ => panic!("unexpected route result request"),
                }
            }
            assert_eq!(poll.request(), None);
            assert_eq!(json!(poll.result().unwrap()), row["returned"]);
        } else {
            let done = movement.stop_route(&mut placement, &mut control).unwrap();
            assert_eq!(json!(i32::from(done)), row["returned"]);
            assert_eq!(
                row["trace"],
                if done {
                    json!([[0x6f6d68, 7]])
                } else {
                    json!([])
                }
            );
        }
        assert_eq!(
            movement.state,
            serde_json::from_value(expected["state"].clone()).unwrap()
        );
        assert_eq!(
            placement,
            serde_json::from_value(expected["placement"].clone()).unwrap()
        );
        assert_eq!(
            control,
            serde_json::from_value(expected["ordinary"].clone()).unwrap()
        );
        assert_eq!(json!(movement.scalar_7c), expected["scalar_7c"]);
        assert_eq!(json!(object22), expected["object22"]);
        assert_eq!(
            json!(movement.path.as_ref().unwrap().stop_mode),
            expected["stop_mode"]
        );
        let mut path: sims_sim::pose::movement::path::Path =
            serde_json::from_value(expected["path"].clone()).unwrap();
        path.stop_mode = expected["stop_mode"].as_i64().unwrap() as i32;
        assert_eq!(movement.path.as_ref(), Some(&path));
        assert_eq!(
            before["animator"], expected["animator"],
            "route command changed layers"
        );
        assert_eq!(
            json!(facing),
            expected["facing"],
            "route command physical facing"
        );
        let restored: (Movement, Placement, ordinary::State) = serde_json::from_slice(
            &serde_json::to_vec(&(&movement, &placement, &control)).unwrap(),
        )
        .unwrap();
        assert_eq!((movement, placement, control), restored);
    }
}
