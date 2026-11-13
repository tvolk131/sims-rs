use serde_json::{Value, json};
use sims_vm::scheduler::clock::draw::actions::tail;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_dirty_vector_and_idle_services() {
    let all: Value =
        serde_json::from_str(include_str!("fixtures/native-action-tail.json")).unwrap();
    assert_eq!(
        all["source_sha256"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    assert_eq!(all["cases"].as_array().unwrap().len(), 252);
    for (i, row) in all["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let kind = c["kind"].as_str().unwrap();
        let actual = match kind {
            "append" | "clear" => {
                let mut header: [u32; 3] = serde_json::from_value(c["header"].clone()).unwrap();
                let words: Vec<u32> = serde_json::from_value(c["backing"].clone()).unwrap();
                let mut backing: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
                let mut growth = vec![];
                if kind == "append" {
                    if let tail::Append::Growth { position, value } = tail::append(
                        &mut header,
                        &mut backing,
                        c["value"].as_u64().unwrap() as u32,
                    )
                    .unwrap()
                    {
                        growth.push(json!({"position":position,"value":value}));
                    }
                } else {
                    tail::clear(&mut header, &backing).unwrap();
                }
                let words: Vec<u32> = backing
                    .chunks_exact(4)
                    .map(|x| u32::from_le_bytes(x.try_into().unwrap()))
                    .collect();
                json!({"header":header,"backing":words,"growth":growth})
            }
            "dirty" => {
                let flags = c["flags"].as_u64().unwrap() as u32;
                json!({"value":tail::caption_dirty(flags),"flags":flags})
            }
            "idle" => {
                let text = c["header"].as_str().unwrap();
                let h: Vec<u8> = (0..text.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                    .collect();
                assert!(tail::idle_advance(&h).unwrap());
                json!({"header":text})
            }
            _ => panic!(),
        };
        assert_eq!(actual, row["expected"], "case{i}");
    }
}
#[test]
fn growth_and_active_animation_require_children_without_mutation() {
    let mut header = [0x100, 0x104, 0x104];
    let mut backing = 7u32.to_le_bytes();
    assert_eq!(
        tail::append(&mut header, &mut backing, 9).unwrap(),
        tail::Append::Growth {
            position: 0x104,
            value: 9
        }
    );
    assert_eq!(header, [0x100, 0x104, 0x104]);
    assert_eq!(backing, 7u32.to_le_bytes());
    let mut h = vec![0; 0x124];
    h[0xf0] = 1;
    assert!(!tail::idle_advance(&h).unwrap());
    let before = (header, backing);
    header[1] = 0x108;
    assert!(tail::append(&mut header, &mut backing, 0).is_err());
    assert_eq!(backing, before.1);
}
