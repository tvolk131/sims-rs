use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_vm::scheduler::clock::draw::actions::{
    Record,
    clear::{Frame, Request},
};
use std::collections::{BTreeMap, BTreeSet};
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}
fn state(records: &[Record], images: &BTreeMap<u32, u32>, buffers: &BTreeSet<u32>) -> Value {
    json!({"records":records.iter().map(|r|hex(r)).collect::<Vec<_>>(),"counters":images,"buffers":buffers})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_full_clear_and_every_resource_child_restore() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/native-action-record-clear.json")).unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 192);
    for (i, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let mut records: Vec<Record> = c["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| bytes(r.as_str().unwrap()).try_into().unwrap())
            .collect();
        let mut images: BTreeMap<u32, u32> = serde_json::from_value(c["counters"].clone()).unwrap();
        let mut buffers: BTreeSet<u32> = serde_json::from_value(c["buffers"].clone()).unwrap();
        let mut frame = Frame::at_entry();
        let mut events = vec![];
        let mut cuts = vec![];
        for _ in 0..100 {
            let Some(request) = frame.advance(&mut records, &mut images).unwrap() else {
                break;
            };
            assert_eq!(
                frame.advance(&mut records, &mut images).unwrap(),
                Some(request)
            );
            cuts.push(hex(&Sha256::digest(
                serde_json::to_vec(&state(&records, &images, &buffers)).unwrap(),
            )));
            let wire = serde_json::to_vec(&(&frame, &records, &images, &buffers)).unwrap();
            let (restored, r, im, b): (Frame, Vec<Record>, BTreeMap<u32, u32>, BTreeSet<u32>) =
                serde_json::from_slice(&wire).unwrap();
            assert_eq!(frame, restored);
            assert_eq!(records, r);
            assert_eq!(images, im);
            assert_eq!(buffers, b);
            frame = restored;
            match request {
                Request::FreeString { pointer, .. } => assert!(buffers.remove(&pointer)),
                Request::DestroyImage { image, .. } => assert_eq!(images[&image], 0),
            }
            frame.acknowledge(&mut records, &images, request).unwrap();
            events.push(serde_json::to_value(request).unwrap());
        }
        assert!(frame.complete());
        assert_eq!(
            json!({"events":events,"cut_sha256":cuts,"state":state(&records,&images,&buffers)}),
            row["expected"],
            "case {i}"
        );
    }
}
