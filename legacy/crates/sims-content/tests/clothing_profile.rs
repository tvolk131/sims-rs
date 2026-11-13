use sims_content::{clothing_catalog::Names, clothing_profile::select_pajamas};
use sims_sim::pose::clothing::texture_names;
use sims_vm::{Definition, Program, body_profiles::State};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_pajama_fixtures_match_with_real_runtime_profile_ownership() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../sims-sim/tests/fixtures/pajama-selector.json"
    ))
    .unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let values: Vec<String> = serde_json::from_value(input["profile"].clone()).unwrap();
        let names = |key: &str| {
            input[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().as_bytes().to_vec())
                .collect()
        };
        let names = Names {
            bodies: names("bodies"),
            textures: texture_names::Cache {
                names: names("textures"),
            },
        };
        let mut program = Program::default();
        let d = Definition {
            source: "person".into(),
            object: 128,
            guid: 1,
            slots: 0,
        };
        let module = program.modules.entry(d.source.clone()).or_default();
        let mut words = vec![0; 20];
        words[9] = 2;
        words[19] = 200;
        for id in [128, 129, 130] {
            module.object_definition_words.insert(id, words.clone());
        }
        module.strings.insert(200, values.clone());
        let mut state = State::default();
        // A second already-loaded definition must not observe publications to
        // the shared bank until it explicitly discards/reloads its own cache.
        let stale = Definition {
            object: 129,
            ..d.clone()
        };
        state.load(&program, &stale).unwrap();
        let fingerprint = program.fingerprint();
        let selected = select_pajamas(
            &program,
            &mut state,
            &d,
            input["person_age"].as_i64().unwrap() as i16,
            &names,
        )
        .unwrap();
        let expected = &row["original"];
        assert_eq!(
            String::from_utf8(selected.body).unwrap(),
            expected["body"],
            "body {i}"
        );
        assert_eq!(
            String::from_utf8(selected.texture).unwrap(),
            expected["texture"],
            "texture {i}"
        );
        assert_eq!(
            serde_json::json!(state.get(&d).unwrap()),
            expected["profile"],
            "profile {i}"
        );
        assert_eq!(state.get(&stale).unwrap(), values);
        let fresh = Definition {
            object: 130,
            ..d.clone()
        };
        assert_eq!(
            serde_json::json!(state.load(&program, &fresh).unwrap().unwrap()),
            expected["profile"],
            "published bank {i}"
        );
        state.validate(&program).unwrap();
        let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(restored, state);
        assert_eq!(program.fingerprint(), fingerprint);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn migration_does_not_reselect_the_new_sleep_descriptor_until_the_next_selection()
-> anyhow::Result<()> {
    use sims_sim::pose::appearance::{Binding, Record, Records};
    use sims_sim::pose::appearance_selection::{self, Request};
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../sims-sim/tests/fixtures/pajama-selector.json"
    ))?;
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let values: Vec<String> = serde_json::from_value(input["profile"].clone())?;
        if !values[32].is_empty() {
            continue;
        }
        let names = |key: &str| {
            input[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().as_bytes().to_vec())
                .collect()
        };
        let names = Names {
            bodies: names("bodies"),
            textures: texture_names::Cache {
                names: names("textures"),
            },
        };
        let mut program = Program::default();
        let d = Definition {
            source: "person".into(),
            object: 128,
            guid: 1,
            slots: 0,
        };
        let module = program.modules.entry(d.source.clone()).or_default();
        let mut words = vec![0; 20];
        words[9] = 2;
        words[19] = 200;
        module.object_definition_words.insert(128, words);
        module.strings.insert(200, values.clone());
        let mut profiles = State::default();
        profiles.load(&program, &d).unwrap();
        let mut person = (0..256)
            .map(|n| (n, 0))
            .collect::<std::collections::BTreeMap<i16, i16>>();
        person.insert(8, 5);
        person.insert(58, input["person_age"].as_i64().unwrap() as i16);
        let mut selection = appearance_selection::State::constructed([0; 3]);
        let mut records = Records::default();
        let mut chosen = sims_content::clothing_profile::select_appearance(
            &program,
            &mut profiles,
            &d,
            &mut selection,
            &mut person,
            0,
            false,
            &names,
        )?;
        assert_eq!(
            serde_json::json!(profiles.get(&d).unwrap()),
            row["original"]["profile"]
        );
        let expected = if row["original"]["body"].as_str().unwrap().is_empty() {
            appearance_selection::descriptor(&values[1], false)
        } else {
            appearance_selection::Descriptor {
                name: row["original"]["body"].as_str().unwrap().into(),
                texture: row["original"]["texture"].as_str().unwrap().into(),
            }
        };
        let mut body = None;
        while let Some(request) = chosen.next(&selection, &records) {
            let saved = serde_json::to_vec(&chosen)?;
            chosen = serde_json::from_slice(&saved)?;
            assert_eq!(chosen.next(&selection, &records), Some(request.clone()));
            if let Request::Bind { record, descriptor } = request {
                if record == 4 {
                    body = Some(descriptor.clone());
                }
                if !descriptor.name.is_empty() {
                    records.entries[record] = Record {
                        binding: Some(Binding {
                            id: record as u64 + 1,
                            owner: 1,
                        }),
                        name: descriptor.name,
                        texture: descriptor.texture,
                    };
                }
            }
            chosen.acknowledge().unwrap();
        }
        assert_eq!(body, Some(expected), "first selection {i}");
        let upgraded = profiles.get(&d).unwrap()[32].clone();
        if !upgraded.is_empty() {
            let before = profiles.clone();
            let mut second = sims_content::clothing_profile::select_appearance(
                &program,
                &mut profiles,
                &d,
                &mut selection,
                &mut person,
                0,
                true,
                &names,
            )?;
            let mut second_body = None;
            // Empty bindings make the selected descriptor observable even if
            // the generated and legacy pajamas happen to name the same asset.
            let empty = Records::default();
            while let Some(request) = second.next(&selection, &empty) {
                if let Request::Bind {
                    record: 4,
                    descriptor,
                } = request
                {
                    second_body = Some(descriptor);
                }
                second.acknowledge().unwrap();
            }
            let mut expected_second = appearance_selection::descriptor(&upgraded, false);
            // A migration may publish ",BODY=" when no assets match. The
            // nonempty bank entry still has an empty parsed body, so 58d492
            // selects the ordinary outfit even on the subsequent call.
            if expected_second.name.is_empty() {
                expected_second = appearance_selection::descriptor(&values[1], false);
            }
            assert_eq!(second_body, Some(expected_second), "second selection {i}");
            assert_eq!(profiles, before);
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires original homes/assets and names emitted by object_resources corpus test"]
fn original_household_profiles_with_ordered_clothing_catalog() -> anyhow::Result<()> {
    use anyhow::ensure;
    use sha2::{Digest, Sha256};
    use sims_content::Index;
    use sims_runtime::original_session::{Options, Session};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let bytes = std::fs::read(root.join("cache/original-clothing-catalog-names.json"))?;
    let names: Names = serde_json::from_slice(&bytes)?;
    let decode = |values: &[Vec<u8>]| {
        values
            .iter()
            .map(|v| sims_formats::strings::decode(v, 1))
            .collect::<Vec<_>>()
    };
    let mut cases = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let session = Session::load(&index, source, Options::default())?;
        for id in session.simulation.characters.keys() {
            let object = &session.simulation.vm.world.objects[id];
            let definition = object.definition.as_ref().unwrap();
            let age = object.person.as_ref().unwrap()[&58];
            let mut state = session.simulation.vm.world.body_profiles.clone();
            let before = state
                .load(&session.program, definition)
                .map_err(anyhow::Error::msg)?
                .unwrap()
                .to_vec();
            let chosen = select_pajamas(&session.program, &mut state, definition, age, &names)?;
            state
                .validate(&session.program)
                .map_err(anyhow::Error::msg)?;
            let restored: State = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
            ensure!(restored == state);
            cases.push(serde_json::json!({
                "house":source,"object":id,"definition":definition,
                "module_sha256":session.program.modules[&definition.source].content_hash,
                "input":{"profile":before,"person_age":age},
                "expected":{"body":sims_formats::strings::decode(&chosen.body,1),
                    "texture":sims_formats::strings::decode(&chosen.texture,1),
                    "profile":state.get(definition).map_err(anyhow::Error::msg)?}
            }));
        }
    }
    ensure!(!cases.is_empty());
    std::fs::write(
        root.join("cache/original-clothing-profile-inputs.json"),
        serde_json::to_vec(&serde_json::json!({
            "scope":"Original loaded resident profiles, live ages and source-backed ordered catalogs. Asset-only startup settings and mounted source order remain explicit inputs; no claim of full native startup or live Reset completion.",
            "catalog_sha256":format!("{:x}",Sha256::digest(&bytes)),
            "bodies":decode(&names.bodies),"textures":decode(&names.textures.names),"cases":cases
        }))?,
    )?;
    println!(
        "{} original resident profile selections; {} bodies, {} textures",
        cases.len(),
        names.bodies.len(),
        names.textures.names.len()
    );
    Ok(())
}
