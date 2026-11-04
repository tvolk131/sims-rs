use serde_json::{Value, json};
use sims_vm::scheduler::clock::{
    draw::{Fields, Frame, Reply, Request},
    fps::History,
    profiler::Buffer,
};

fn history(value: &Value) -> History {
    let fields: [u32; 4] = serde_json::from_value(value["fields"].clone()).unwrap();
    History::from_captured_parts(
        serde_json::from_value(value["timestamps"].clone()).unwrap(),
        fields[0],
        fields[1],
        fields[2],
        fields[3],
    )
    .unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn unchanged_native_body_call_order_late_reads_and_every_suspended_cursor()
-> Result<(), Box<dyn std::error::Error>> {
    let native: Value =
        serde_json::from_str(include_str!("fixtures/native-application-draw.json"))?;
    assert_eq!(native["cases"].as_array().unwrap().len(), 24);
    for case in native["cases"].as_array().unwrap() {
        let mut fields: Fields = serde_json::from_value(case["before"].clone())?;
        let mut frame = Frame::at_entry();
        let mut fps = history(&case["history_before"]);
        let mut profiler = Buffer::from_captured(vec![], 32)?;
        let mut events = Vec::new();
        while let Some(request) = frame.advance(&mut fields)? {
            let saved = serde_json::to_vec(&(&fields, &frame, &fps, &profiler))?;
            (fields, frame, fps, profiler) = serde_json::from_slice(&saved)?;
            assert_eq!(frame.advance(&mut fields)?, Some(request));
            assert_eq!(
                serde_json::to_vec(&(&fields, &frame, &fps, &profiler))?,
                saved
            );
            let wrong = if matches!(request, Request::FpsClock) {
                Reply::Done
            } else {
                Reply::Clock(0)
            };
            assert!(
                frame
                    .acknowledge(request, wrong, &mut fps, &mut profiler)
                    .is_err()
            );
            assert_eq!(
                serde_json::to_vec(&(&fields, &frame, &fps, &profiler))?,
                saved
            );
            let expected = &case["events"][events.len()];
            let (event, reply) = match request {
                Request::Timestamp { name } => (
                    json!({"kind":"timestamp","name":name,"value":expected["value"]}),
                    Reply::Timestamp(expected["value"].as_u64().unwrap()),
                ),
                Request::FpsClock => (
                    json!({"kind":"fps_clock","value":expected["value"]}),
                    Reply::Clock(expected["value"].as_u64().unwrap() as u32),
                ),
                Request::ViewBegin { owner } => {
                    (json!({"kind":"view_begin","owner":owner}), Reply::Done)
                }
                Request::ApplicationView => {
                    if let Some(owner) = case["mutation"]["unload"].as_u64() {
                        fields.unload = Some(owner as u32);
                    }
                    (
                        json!({"kind":"application_view","owner":0x10000000u32}),
                        Reply::Done,
                    )
                }
                Request::Unload { owner } => (json!({"kind":"unload","owner":owner}), Reply::Done),
                Request::Framework { owner } => {
                    (json!({"kind":"framework","owner":owner}), Reply::Done)
                }
                Request::World { owner, argument } => {
                    if let Some(owner) = case["mutation"]["view"].as_u64() {
                        fields.view = Some(owner as u32);
                    }
                    (
                        json!({"kind":"world","owner":owner,"argument":argument}),
                        Reply::Done,
                    )
                }
                Request::ViewEnd { owner } => {
                    (json!({"kind":"view_end","owner":owner}), Reply::Done)
                }
            };
            assert_eq!(&event, expected, "{}", case["label"]);
            events.push(event);
            frame.acknowledge(request, reply, &mut fps, &mut profiler)?;
        }
        assert!(frame.complete());
        assert_eq!(serde_json::to_value(fields)?, case["after"]);
        assert_eq!(fps, history(&case["history_after"]));
        assert_eq!(json!(events), case["events"]);
        assert_eq!(
            json!(
                profiler
                    .records()
                    .iter()
                    .map(|r| r.words())
                    .collect::<Vec<_>>()
            ),
            case["profiler_records"]
        );
    }
    Ok(())
}

#[test]
fn profiler_allocation_failure_retains_the_pending_timestamp_and_history()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fields = Fields {
        busy: 0,
        view: None,
        unload: None,
        framework: 1,
        world: 2,
    };
    let mut frame = Frame::at_entry();
    let request = frame.advance(&mut fields)?.unwrap();
    let mut history = History::from_captured_parts(vec![0; 1000], 1000, 0, 0, 0)?;
    let mut profiler = Buffer::from_captured(vec![], 0)?;
    let before = serde_json::to_vec(&(&fields, &frame, &history, &profiler))?;
    assert!(
        frame
            .acknowledge(request, Reply::Timestamp(99), &mut history, &mut profiler)
            .is_err()
    );
    assert_eq!(
        serde_json::to_vec(&(&fields, &frame, &history, &profiler))?,
        before
    );
    let mut malformed = serde_json::to_value(&frame)?;
    malformed["phase"] = json!("Complete");
    let mut malformed: Frame = serde_json::from_value(malformed)?;
    assert!(malformed.advance(&mut fields).is_err());
    Ok(())
}
