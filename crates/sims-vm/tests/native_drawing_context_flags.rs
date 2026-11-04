use serde_json::Value;
use sims_vm::scheduler::clock::draw::context::{Frame, Reply};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_flag_setter_low_byte_masks_groups_getters_and_resume() {
    let v: Value =
        serde_json::from_str(include_str!("fixtures/native-drawing-context-flags.json")).unwrap();
    let rows = v["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 256);
    let mut checks = 0;
    for row in rows {
        let mut frame = Frame::new(0x10001000);
        let before = frame.clone();
        assert!(frame.update_flags(0, 15, 1).is_err());
        assert_eq!(frame, before);
        for i in 0..7 {
            let request = frame.advance().unwrap().unwrap();
            let v = &row["input"]["values"][i];
            frame
                .acknowledge(
                    request,
                    if i == 4 {
                        Reply::Origin([
                            v[0].as_u64().unwrap() as u32,
                            v[1].as_u64().unwrap() as u32,
                        ])
                    } else {
                        Reply::Word(v.as_u64().unwrap() as u32)
                    },
                )
                .unwrap();
        }
        assert_eq!(serde_json::json!(frame.state().words()), row["constructed"]);
        for event in row["events"].as_array().unwrap() {
            frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            frame.validate().unwrap();
            assert_eq!(serde_json::json!(frame.state().words()), event["before"]);
            let before = frame.clone();
            assert!(frame.update_flags(7, 1, 1).is_err());
            assert_eq!(frame, before);
            frame
                .update_flags(
                    event["group"].as_u64().unwrap() as u32,
                    event["mask"].as_u64().unwrap() as u32,
                    event["enabled_word"].as_u64().unwrap() as u8,
                )
                .unwrap();
            assert_eq!(serde_json::json!(frame.state().words()), event["after"]);
            let flags = frame.state().flags(0).unwrap();
            let other = frame.state().flags(1).unwrap();
            assert_eq!(
                serde_json::json!([flags & 1, (flags >> 1) & 1, (flags >> 3) & 1, other & 1]),
                event["getters"]
            );
            frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            frame.validate().unwrap();
            checks += 1;
        }
    }
    assert_eq!(checks, 2048);
}
