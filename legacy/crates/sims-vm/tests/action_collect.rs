use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_vm::scheduler::clock::draw::actions::{
    Record,
    collect::{Frame, Reply, Request},
};
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}
fn word(r: &mut Record, offset: usize, value: u32) {
    r[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn logical(frame: &Frame, output: &[Record]) -> Value {
    json!({"temporary":frame.temporary().map(|r|hex(r)),"output":output.iter().map(|r|hex(r)).collect::<Vec<_>>()})
}
fn event(request: &Request) -> Value {
    let mut v = serde_json::to_value(request).unwrap();
    if let Some(m) = v.as_object_mut() {
        for x in m.values_mut() {
            if let Some(r) = x.get_mut("record") {
                let b: Vec<u8> = r
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                *r = json!(hex(&b));
            }
        }
    }
    v
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_collection_caller_reuses_temp_and_restores_every_child() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/native-action-collect.json")).unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 144);
    for (n, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let mut frame = Frame::at_entry(326);
        let mut output: Vec<Record> = c["initial_output"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| bytes(v.as_str().unwrap()).try_into().unwrap())
            .collect();
        let (mut events, mut cuts, mut builds) = (vec![], vec![], 0);
        for _ in 0..100 {
            let Some(request) = frame.advance().unwrap() else {
                break;
            };
            assert_eq!(frame.advance().unwrap(), Some(request.clone()));
            cuts.push(hex(&Sha256::digest(
                serde_json::to_vec(&logical(&frame, &output)).unwrap(),
            )));
            events.push(event(&request));
            let wire = serde_json::to_vec(&frame).unwrap();
            frame = serde_json::from_slice(&wire).unwrap();
            frame.validate().unwrap();
            let reply = match &request {
                Request::Person { index } => {
                    assert_eq!(*index, 326);
                    Reply::Person(c["person"].as_bool().unwrap().then_some(0x10001000))
                }
                Request::Construct => {
                    Reply::Record(bytes(c["initial"].as_str().unwrap()).try_into().unwrap())
                }
                Request::Build {
                    index,
                    person,
                    node,
                    current,
                    record,
                } => {
                    let success = if *current != 0 {
                        c["active"].as_bool().unwrap()
                    } else {
                        c["queued"][((*node - *person - 0x820) / 0x44) as usize]
                            .as_bool()
                            .unwrap()
                    };
                    let mut r = *record;
                    if success {
                        word(&mut r, 0, *index);
                        word(&mut r, 4, 700 + builds);
                        r[8] = 1;
                        r[9] = if *current != 0 { 1 } else { 9 };
                        word(&mut r, 12, *node);
                    }
                    builds += 1;
                    Reply::Built { record: r, success }
                }
                Request::Append { record } => Reply::Record(*record),
                Request::QueueCount { .. } => Reply::Count(c["count"].as_i64().unwrap() as i32),
                Request::QueueAction { person, index } => {
                    Reply::Action(person + 0x820 + index * 0x44)
                }
                Request::DestroyString { .. } => Reply::Done,
                Request::DestroyImage { record } => {
                    let mut r = *record;
                    r[4..8].fill(0);
                    Reply::Record(r)
                }
            };
            frame.acknowledge(&mut output, request, reply).unwrap();
        }
        assert!(frame.complete());
        assert_eq!(
            json!({"events":events,"cut_sha256":cuts,"state":logical(&frame,&output)}),
            row["expected"],
            "case {n}"
        );
    }
}
