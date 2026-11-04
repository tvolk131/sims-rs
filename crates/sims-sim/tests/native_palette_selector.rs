use sims_sim::lot_presenter::palette_select::Selection;
#[cfg(feature = "private-fixtures")]
#[test]
fn selector_matches_original_returns_requests_and_suspended_frames() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-palette-selector.json")).unwrap();
    assert_eq!(all["source_unchanged"], true);
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 108);
    let get = |v: &serde_json::Value, k: &str| v[k].as_u64().unwrap() as u32;
    for (n, c) in cases.iter().enumerate() {
        let mut state = Selection::new(
            get(&all, "owner"),
            get(c, "light"),
            get(c, "unit"),
            get(c, "epsilon"),
            get(c, "cached"),
            get(c, "cache_generation"),
            get(c, "current_generation"),
        )
        .unwrap();
        for cut in c["cuts"].as_array().unwrap() {
            let pc = get(cut, "pc");
            let expected = if pc == get(&all, "return_pc") {
                0x4c1bc7
            } else {
                pc
            };
            assert_eq!(state.pc(), expected, "case{n}");
            assert_eq!(
                state.esp_delta() as i64,
                cut["esp_delta"].as_i64().unwrap(),
                "case{n} stack"
            );
            if let Some(eax) = state.eax() {
                assert_eq!(eax, get(cut, "eax"), "case{n} return pointer");
            }
            let encoded = serde_json::to_vec(&state).unwrap();
            let mut restored: Selection = serde_json::from_slice(&encoded).unwrap();
            let a = state.advance().unwrap();
            let b = restored.advance().unwrap();
            assert_eq!(state, restored);
            assert_eq!(a, b);
        }
        let writes = state
            .cache_write()
            .map(|v| vec![serde_json::json!({"offset":24,"bytes":4,"value":v[1]})])
            .unwrap_or_default();
        assert_eq!(
            serde_json::json!(writes),
            c["writes"],
            "case{n} cache stores"
        );
        if c["result"].is_null() {
            assert!(state.pending().is_some());
            assert!(!state.advance().unwrap());
        } else {
            assert!(state.complete());
            assert_eq!(state.result(), Some(get(c, "result")));
        }
    }
}
