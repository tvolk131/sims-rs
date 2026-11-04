use sims_sim::lot_presenter::level_geometry::{Frame, Words};
use std::collections::BTreeMap;
struct Memory(BTreeMap<u32, u32>);
impl Words for Memory {
    fn word(&self, address: u32) -> Result<u32, String> {
        self.0
            .get(&address)
            .copied()
            .ok_or_else(|| format!("unknown {address:x}"))
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn iterator_matches_original_order_and_live_header_changes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-level-geometry-iterator.json")).unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 512);
    for (n, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let mut memory = Memory(
            serde_json::from_value::<Vec<(u32, u32)>>(input["words"].clone())
                .unwrap()
                .into_iter()
                .collect(),
        );
        let mut frame = Frame::new(
            input["model"].as_u64().unwrap() as u32,
            input["level"].as_i64().unwrap() as i32,
        );
        let mut calls = vec![];
        for _ in 0..256 {
            if let Some(r) = frame.pending().cloned() {
                calls.push(serde_json::json!({"function":r.kind.entry(),"model":r.model,"cursor":frame.cursor(),"header":frame.header()}));
                for action in input["actions"].as_array().unwrap() {
                    if action["after_call"] == calls.len() {
                        for a in action["writes"].as_array().unwrap() {
                            memory.0.insert(
                                a[0].as_u64().unwrap() as u32,
                                a[1].as_u64().unwrap() as u32,
                            );
                        }
                    }
                }
                let mut cold: Frame =
                    serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
                cold.acknowledge(&r).unwrap();
                frame.acknowledge(&r).unwrap();
                assert_eq!(cold.advance(&memory), frame.advance(&memory));
                assert_eq!(cold, frame, "cold case{n}");
            } else if !frame.advance(&memory).unwrap() {
                break;
            }
        }
        assert!(frame.complete(), "case{n} did not complete");
        assert_eq!(serde_json::json!(calls), row["original"], "case{n}");
    }
}
