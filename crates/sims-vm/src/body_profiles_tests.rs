use super::*;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_definition_caches_preserve_bank_publication_and_restore_order() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/body-profile-cache.json")).unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut program = Program::default();
        let mut definitions = vec![];
        let length = input["length"].as_u64().unwrap() as usize;
        for (id, def) in input["definitions"].as_array().unwrap().iter().enumerate() {
            let source = def["file"].to_string();
            let bank = def["bank"].as_u64().unwrap() as u16;
            let module = program.modules.entry(source.clone()).or_default();
            let mut words = vec![0; 20];
            words[9] = def["kind"].as_i64().unwrap() as i16;
            words[19] = bank as i16;
            module.object_definition_words.insert(id as u16, words);
            module
                .strings
                .insert(bank, (1..=length).map(|n| format!("entry-{n}")).collect());
            definitions.push(Definition {
                source,
                object: id as u16,
                guid: id as u32,
                slots: 0,
            });
        }
        let original = program.fingerprint();
        let mut state = State::default();
        let mut cache_ids = BTreeMap::new();
        let mut result = vec![];
        for step in input["steps"].as_array().unwrap() {
            let id = step["definition"].as_u64().unwrap() as usize;
            let d = &definitions[id];
            result.push(match step["op"].as_str().unwrap() {
                "get" => match state.load(&program, d).unwrap() {
                    None => serde_json::Value::Null,
                    Some(values) => {
                        let next_id = cache_ids.len();
                        let cache = *cache_ids.entry(id).or_insert(next_id);
                        serde_json::json!({ "cache": cache, "profile": values })
                    }
                },
                "edit" => {
                    state.get_mut(d).unwrap()[1] = step["value"].as_str().unwrap().into();
                    serde_json::Value::Null
                }
                "publish" => {
                    state.publish(d).unwrap();
                    serde_json::Value::Null
                }
                op => panic!("unexpected cache operation {op}"),
            });
            state.validate(&program).unwrap();
            // Preserve cache divergence across every possible suspension point.
            let restored: State =
                serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            assert_eq!(restored, state);
            state = restored;
            assert_eq!(program.fingerprint(), original, "modified original program");
        }
        assert_eq!(
            serde_json::json!(result),
            row["original"]["steps"],
            "cache history {case}"
        );
        for expected in row["original"]["banks"].as_array().unwrap() {
            let source = expected["file"].to_string();
            let bank = expected["bank"].as_u64().unwrap() as u16;
            let actual = state
                .banks
                .get(&source)
                .and_then(|m| m.get(&bank))
                .unwrap_or(&program.modules[&source].strings[&bank]);
            assert_eq!(
                serde_json::json!(actual),
                expected["profile"],
                "published bank {case}"
            );
        }
        let mut corrupt = state.clone();
        corrupt
            .definitions
            .get_mut(&definitions[0].source)
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .bank = 999;
        assert!(corrupt.validate(&program).is_err());
        let mut corrupt = state.clone();
        corrupt.get_mut(&definitions[0]).unwrap()[0] = "allowed runtime edit".into();
        corrupt.validate(&program).unwrap();
        corrupt
            .definitions
            .get_mut(&definitions[0].source)
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .values
            .clear();
        assert!(corrupt.validate(&program).is_err());
    }
}
