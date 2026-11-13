use serde_json::Value;
use sims_vm::scheduler::clock::draw::context::{Frame, Reply};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_context_getter_order_partial_fields_wrapping_and_cold_resume() {
    let native: Value =
        serde_json::from_str(include_str!("fixtures/native-drawing-context.json")).unwrap();
    let rows = native["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 256);
    for row in rows {
        let mut frame = Frame::new(0x10001000);
        for (i, event) in row["expected"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            let request = frame.advance().unwrap().unwrap();
            assert_eq!(request.owner, event["owner"].as_u64().unwrap() as u32);
            assert_eq!(
                request.getter.slot(),
                event["slot"].as_u64().unwrap() as u32
            );
            assert_eq!(serde_json::json!(frame.state().words()), event["before"]);
            let before = frame.clone();
            let wrong = if i == 4 {
                Reply::Word(0)
            } else {
                Reply::Origin([0; 2])
            };
            assert!(frame.acknowledge(request, wrong).is_err());
            assert_eq!(frame, before);
            frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            assert_eq!(frame.advance().unwrap(), Some(request));
            let v = &row["input"]["values"][i];
            let reply = if i == 4 {
                Reply::Origin([v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32])
            } else {
                Reply::Word(v.as_u64().unwrap() as u32)
            };
            frame.acknowledge(request, reply).unwrap();
        }
        frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
        assert!(frame.complete());
        assert_eq!(frame.advance().unwrap(), None);
        assert_eq!(
            serde_json::json!(frame.state().words()),
            row["expected"]["words"]
        );
    }
}
