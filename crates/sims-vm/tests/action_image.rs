use serde_json::{Value, json};
use sims_vm::scheduler::clock::draw::actions::image::{Frame, Request};
use std::collections::BTreeMap;
const W: u32 = 0x10000100;
fn image(i: u32) -> u32 {
    if i == 0 {
        0
    } else {
        0x10002000 + (i - 1) * 256
    }
}
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn state(h: &[u8], counts: &BTreeMap<u32, u32>) -> Value {
    json!({"header":h.iter().map(|x|format!("{x:02x}")).collect::<String>(),"counts":counts})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_image_publication_and_every_child_restore() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/native-action-image.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 256);
    for (i, row) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut h = bytes(c["header"].as_str().unwrap());
        let mut counts = (1..=3)
            .map(|id| {
                (
                    image(id),
                    c["counts"][id as usize - 1].as_u64().unwrap() as u32,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let sizes = (1..=3)
            .map(|id| {
                (
                    image(id),
                    [
                        c["size"][0].as_i64().unwrap() as i32,
                        c["size"][1].as_i64().unwrap() as i32,
                    ],
                )
            })
            .collect();
        let mut frame = Frame::new(W, image(c["image"].as_u64().unwrap() as u32));
        for event in row["expected"]["events"].as_array().unwrap() {
            let request = frame.advance(&mut h, &mut counts, &sizes).unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(&request).unwrap(),
                event["request"],
                "request {i}"
            );
            assert_eq!(state(&h, &counts), event["state"], "cut {i}");
            let mut cold: Frame =
                serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            assert_eq!(
                cold.advance(&mut h, &mut counts, &sizes).unwrap(),
                Some(request.clone())
            );
            assert_eq!(cold, frame);
            if let Request::Rectangle { rectangle, .. } = &request {
                for (j, x) in rectangle.iter().enumerate() {
                    h[0x7c + j * 4..0x80 + j * 4].copy_from_slice(&x.to_le_bytes());
                }
            }
            frame.acknowledge(&h, &counts, &request).unwrap();
            let cold: Frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            assert_eq!(frame, cold);
        }
        assert_eq!(frame.advance(&mut h, &mut counts, &sizes).unwrap(), None);
        assert!(frame.complete());
        assert_eq!(state(&h, &counts), row["expected"]["state"], "final {i}");
    }
}
