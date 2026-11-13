//! Original instruction controls, not tick credit.
use sims_sim::lot_presenter::physical_next::Next;
#[cfg(feature = "private-fixtures")]
#[test]
fn physical_next_matches_original_signed_lookup_and_suspended_frames() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-physical-next.json")).unwrap();
    assert_eq!(all["source_unchanged"], true);
    assert_eq!(all["native_parity_credit"], 0);
    let cases = all["cases"].as_array().unwrap();
    let mut cuts = 0;
    for (n, c) in cases.iter().enumerate() {
        let child = c["child"].as_i64().unwrap() as i16;
        let begin = c["table_begin"].as_u64().unwrap() as u32;
        let entries: Vec<u32> = serde_json::from_value(c["table"].clone()).unwrap();
        let table = (child > 0).then_some([begin, begin + entries.len() as u32 * 4]);
        let mut state = Next::new(child, table).unwrap();
        let mut lookups = 0;
        for q in c["cuts"].as_array().unwrap() {
            assert_eq!(state.pc() as u64, q["pc"].as_u64().unwrap(), "case{n}");
            if let Some(eax) = state.eax() {
                assert_eq!(eax as u64, q["eax"].as_u64().unwrap(), "case{n} EAX");
            }
            assert_eq!(
                state.esp_delta() as i64,
                q["esp_delta"].as_i64().unwrap(),
                "case{n} ESP"
            );
            if state.registry_scope() {
                assert_eq!(q["ecx"], c["owner"], "case{n} registry scope");
            }
            let encoded = serde_json::to_vec(&state).unwrap();
            let mut restored: Next = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(state, restored);
            if !state.complete() {
                let a = state
                    .advance(|id| {
                        lookups += 1;
                        Ok(Some(entries[id as usize - 1]))
                    })
                    .unwrap();
                let b = restored
                    .advance(|id| Ok(Some(entries[id as usize - 1])))
                    .unwrap();
                assert!(a && b);
                assert_eq!(state, restored, "case{n} suspended frame");
            }
            cuts += 1;
        }
        assert_eq!(
            state.result().unwrap() as u64,
            c["result"].as_u64().unwrap()
        );
        assert_eq!(
            lookups,
            usize::from(child > 0 && child as usize <= entries.len())
        );
    }
    assert_eq!(cases.len(), 105);
    assert_eq!(cuts, 1164);
}
#[test]
fn unknown_in_range_entry_stays_pending() {
    let mut state = Next::new(1, Some([4096, 4100])).unwrap();
    while state.pc() != 0x407458 {
        assert!(state.advance(|_| panic!("lookup before load")).unwrap());
    }
    let before = state.clone();
    assert!(!state.advance(|_| Ok(None)).unwrap());
    assert_eq!(state, before);
    assert!(state.advance(|_| Ok(Some(123))).unwrap());
    assert_eq!(state.eax(), Some(123));
}
