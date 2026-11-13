//! Original machine-code vectors, separate from completed household ticks.
use sims_sim::lot_presenter::contained_slot::{EMPTY, Empty};
fn bytes(v: &serde_json::Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_empty_slot_lookup_return_and_atomic_output_stores() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-empty-contained-slot.json")).unwrap();
    assert_eq!(all["source_unchanged"], true);
    assert_eq!(all["native_parity_credit"], 0);
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 32);
    let mut total = 0;
    for (n, c) in cases.iter().enumerate() {
        assert_eq!(c["context_and_parent_and_slot_unchanged"], true);
        let mut frame = Empty::new(c["child"].as_i64().unwrap() as i16).unwrap();
        let mut writes = Vec::new();
        for cut in c["cuts"].as_array().unwrap() {
            let pc = cut["pc"].as_u64().unwrap() as u32;
            for _ in 0..32 {
                if frame.pc() == pc {
                    break;
                }
                let at = frame.pc();
                let before = frame.output_words();
                assert!(frame.advance().unwrap());
                for (index, word) in frame.output_words().iter().enumerate() {
                    if before[index] != *word {
                        writes.push(serde_json::json!({"pc":at,"offset":index*4,"value":word.unwrap()as u32}));
                    }
                }
            }
            assert_eq!(frame.pc(), pc, "case{n}");
            let native = bytes(&cut["output"]);
            for (index, word) in frame.output_words().iter().enumerate() {
                if let Some(word) = word {
                    assert_eq!(
                        word.to_le_bytes(),
                        native[index * 4..index * 4 + 4],
                        "case{n} output{index}"
                    );
                }
            }
            if frame.lookup_returned() {
                assert_eq!(cut["eax"], 0);
            }
            //Every original boundary can be reconstructed as suspended ordinary data.
            let encoded = serde_json::to_vec(&frame).unwrap();
            let restored: Empty = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(frame, restored);
            restored.validate().unwrap();
        }
        assert_eq!(
            serde_json::to_value(&writes).unwrap(),
            c["writes"],
            "case{n} ordered stores"
        );
        while !frame.complete() {
            assert!(frame.advance().unwrap());
        }
        assert_eq!(frame.result(), Some(EMPTY));
        assert_eq!(
            bytes(&c["result"]),
            EMPTY
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect::<Vec<_>>()
        );
        total += writes.len();
    }
    assert_eq!(total, 128);
    assert!(Empty::new(1).is_err());
}
