use sims_vm::interface::drawing::Change;

#[test]
fn drawing_publishes_low_byte_once_before_resumable_broadcast() {
    for before in [0, 1, 128, 255] {
        for argument in [0, 1, 128, 255, 0x100, 0x101, 0x80000080, 0xffffffff] {
            let mut drawing = before;
            let mut c = Change::new(argument);
            let notify = c.advance(&mut drawing).unwrap();
            assert_eq!(drawing, argument as u8);
            assert_eq!(notify, before != argument as u8);
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            //The native caller does not overwrite a callback's mutation on return.
            drawing = 17;
            assert_eq!(c.advance(&mut drawing).unwrap(), notify);
            assert_eq!(drawing, 17);
            if notify {
                c.acknowledge().unwrap();
            }
            assert!(c.complete());
            let saved = c.clone();
            assert!(c.acknowledge().is_err());
            assert_eq!(saved, c);
        }
    }
}

#[test]
#[ignore = "private guarded4c4222 corpus"]
fn original_drawing_setter_byte_and_notification_match() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/mode-notifications.json");
    let corpus: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(corpus["native_execution"], true);
    let rows = corpus["drawing"].as_array().unwrap();
    assert_eq!(rows.len(), 32);
    for row in rows {
        let mut drawing = row["before"].as_u64().unwrap() as u8;
        let mut c = Change::new(row["argument"].as_u64().unwrap() as u32);
        let mut events = vec![];
        if c.advance(&mut drawing).unwrap() {
            events.push(drawing);
            c.acknowledge().unwrap();
        }
        assert!(c.complete());
        assert_eq!(
            serde_json::json!({"notifications":events,"drawing":drawing}),
            row["original"]
        );
    }
}
