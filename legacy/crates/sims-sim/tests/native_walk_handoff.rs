use serde_json::{Value, json};
use sims_sim::{
    contention::animator_can_interrupt,
    pose::{
        movement::{
            Movement,
            route::{Poll, Stop},
        },
        ordinary,
        transform::Placement,
    },
};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_route_gate_stop_choice_yield_and_retained_outcome() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-walk-handoff.json")).unwrap();
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/native-movement-route-control.json")).unwrap();
    let before = &baseline["cases"][0]["before"];
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 1216);
    for row in fixture["cases"].as_array().unwrap() {
        let c = &row["input"];
        let native = &row["original"];
        let mut movement = Movement::new(serde_json::from_value(before["state"].clone()).unwrap());
        movement.path = Some(serde_json::from_value(before["path"].clone()).unwrap());
        movement.state.status = c["animator"].as_i64().unwrap() as i32;
        let path = movement.path.as_mut().unwrap();
        path.index = 17;
        path.deadline = 19;
        path.stop_mode = 23;
        path.interrupt = 0;
        let mut placement: Placement = serde_json::from_value(before["placement"].clone()).unwrap();
        placement.base = [40.25, 56.5, 0.75];
        placement.previous = [0.; 3];
        let mut control: ordinary::State =
            serde_json::from_value(before["ordinary"].clone()).unwrap();
        control.mode = c["mode"].as_i64().unwrap() as i32;
        let start = c["state"].as_u64().unwrap();
        let mut states = vec![start];
        let mut events = vec![];
        let mut result = 13;
        let mut stop = match start {
            9 => Some(Stop::Success),
            10 => Some(Stop::Retry),
            11 => Some(Stop::Failure),
            _ => None,
        };
        if stop.is_none() {
            events.push("interrupt");
            let requested = c["interrupted"] == true;
            if requested {
                events.push("gate");
            }
            let effective = requested
                && animator_can_interrupt(
                    c["mode"].as_u64().unwrap() as u32,
                    c["clip"].as_i64().map(|v| v as i32),
                );
            stop = Stop::before_poll(effective, c["path"] == true, &mut result);
            if stop.is_none() {
                events.push("0x589d47");
                let poll = Poll::begin(&movement, &mut placement, &mut 0, [40, 56], 1).unwrap();
                assert!(poll.request().is_none());
                stop = Stop::after_poll(poll.result().unwrap()).unwrap();
            }
            if let Some(stop) = stop {
                states.push(match stop {
                    Stop::Success => 9,
                    Stop::Retry => 10,
                    Stop::Failure => 11,
                    Stop::Refresh => 15,
                });
            }
        }
        // Native state 9/10/11 resumes after a yield without rechecking the
        // interruption predicate or consulting the current foreground gate.
        (stop, movement, placement, control) = serde_json::from_slice(
            &serde_json::to_vec(&(stop, &movement, &placement, &control)).unwrap(),
        )
        .unwrap();
        let mut yielded = true;
        if let Some(stop) = stop {
            events.push("0x589e2d");
            if movement.stop_route(&mut placement, &mut control).unwrap() {
                events.push("end_route");
                yielded = false;
                states.push(match stop {
                    Stop::Success => 8,
                    Stop::Retry => 6,
                    Stop::Failure => 7,
                    Stop::Refresh => 1,
                });
            }
        }
        assert_eq!(json!(states), native["states"], "{c}");
        assert_eq!(json!(events), native["events"], "{c}");
        assert_eq!(json!(states.last().unwrap()), native["state"], "{c}");
        assert_eq!(json!(yielded), native["yielded"], "{c}");
        assert_eq!(json!(result), native["result"], "{c}");
        let path = movement.path.as_ref().unwrap();
        for (offset, value) in [
            ("0x180", path.index),
            ("0x188", movement.state.status),
            ("0x18c", path.stop_mode),
            ("0x190", control.mode),
            ("0x1c0", path.deadline),
            ("0x284", path.interrupt),
        ] {
            assert_eq!(
                json!(value as u32),
                native["animator"][offset],
                "{c}: {offset}"
            );
        }
        for (i, offset) in ["0x1c4", "0x1c8", "0x1cc"].iter().enumerate() {
            assert_eq!(
                json!(placement.previous[i].to_bits()),
                native["animator"][offset],
                "{c}: {offset}"
            );
        }
        for (key, value) in [
            ("stage", 2),
            ("person72", 0),
            ("data9", 7),
            ("blocked_object", 21),
        ] {
            assert_eq!(native[key], json!(value), "{c}");
        }
    }
}
