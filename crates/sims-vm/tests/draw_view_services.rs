use serde_json::Value;
use sims_vm::scheduler::clock::draw::view::{Next, View, unload_returns};
fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
fn word(raw: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap())
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_view_and_idle_unload_branches_preserve_all_other_supplied_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let native: Value =
        serde_json::from_str(include_str!("fixtures/native-draw-view-services.json"))?;
    assert_eq!(native["cases"].as_array().unwrap().len(), 72);
    let mut count = [0; 3];
    for case in native["cases"].as_array().unwrap() {
        let before = bytes(&case["before"]);
        let mut after = before.clone();
        match case["kind"].as_str().unwrap() {
            "view" => {
                count[0] += 1;
                let mut view = View {
                    mode: word(&before, 4),
                    frame_word: word(&before, 0x38),
                };
                let next = view.begin();
                assert_eq!(next == Next::Return, case["returned"].as_bool().unwrap());
                assert_eq!(view.mode, word(&before, 4));
                after[0x38..0x3c].copy_from_slice(&view.frame_word.to_le_bytes());
            }
            "unload" => {
                count[1] += 1;
                assert_eq!(
                    unload_returns(word(&before, 0x100)),
                    case["returned"].as_bool().unwrap()
                );
            }
            "view_end" => {
                count[2] += 1;
                assert_eq!(case["returned"], true);
            }
            _ => unreachable!(),
        }
        assert_eq!(after, bytes(&case["after"]));
        assert_eq!(case["source_unchanged"], true);
    }
    assert_eq!(count, [32, 32, 8]);
    Ok(())
}
