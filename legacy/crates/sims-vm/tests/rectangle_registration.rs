use serde_json::{Value, json};
use sims_vm::scheduler::clock::draw::regions::{Frame, List, Request};
fn hex(v: &str) -> Vec<u8> {
    (0..v.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&v[i..i + 2], 16).unwrap())
        .collect()
}
fn state(list: &List) -> Value {
    let h: String = list.header.iter().map(|x| format!("{x:02x}")).collect();
    let b: Vec<u32> = list
        .backing
        .chunks_exact(4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .collect();
    json!({"header":h,"backing":b})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_rectangle_registration_each_child_and_backing() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/native-rectangle-registration.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 256);
    for (i, row) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let words: Vec<u32> = serde_json::from_value(c["backing"].clone()).unwrap();
        let mut list = List {
            header: hex(c["header"].as_str().unwrap()),
            backing: words.into_iter().flat_map(u32::to_le_bytes).collect(),
        };
        let mut frame = Frame::new(serde_json::from_value(c["rectangle"].clone()).unwrap());
        let enabled = c["enabled"].as_u64().unwrap() as u32;
        let mut events = vec![];
        while let Some(r) = frame.advance(&mut list, enabled).unwrap() {
            let (restored_frame, restored_list): (Frame, List) =
                serde_json::from_slice(&serde_json::to_vec(&(&frame, &list)).unwrap()).unwrap();
            assert_eq!(restored_frame, frame);
            assert_eq!(restored_list, list);
            let mut restored = restored_frame;
            let mut restored_list = restored_list;
            assert_eq!(
                restored.advance(&mut restored_list, enabled).unwrap(),
                Some(r.clone())
            );
            assert_eq!(restored_list, list);
            events.push(json!({"request":r,"state":state(&list)}));
            match r {
                Request::Append { rectangle } => assert!(list.append(rectangle).unwrap()),
                Request::Coalesce => list.header[12] = 1,
            }
            frame.acknowledge(&mut list, &r).unwrap();
        }
        assert_eq!(
            json!({"events":events,"state":state(&list),"result":frame.result().unwrap()}),
            row["expected"],
            "case{i}"
        );
    }
}
