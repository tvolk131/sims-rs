use serde_json::{Value, json};
use sims_content::clothing_catalog::{Names, Provider};
use sims_vm::{Definition, Program, body_profiles::State, career_appearance};

struct Catalog {
    names: Names,
    calls: usize,
}
impl Provider for Catalog {
    fn names(&mut self) -> anyhow::Result<&Names> {
        self.calls += 1;
        Ok(&self.names)
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_career_uniform_selection_cache_and_profile_migrations() -> anyhow::Result<()> {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-career-appearance.json"))?;
    let constructed = career_appearance::State::default();
    constructed.validate().unwrap();
    for bytes in [vec![b'x'; 64], vec![0]] {
        let mut invalid = constructed.clone();
        invalid.body = bytes;
        assert!(invalid.validate().is_err());
    }
    assert_eq!(
        json!({"key":constructed.key,"lengths":[constructed.body.len(),constructed.texture.len(),constructed.extra.len()]}),
        report["constructor_cache"]
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
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
        let values: Vec<String> = serde_json::from_value(c["profile"].clone())?;
        module.strings.insert(200, values);
        let mut profiles = State::default();
        profiles.load(&program, &d).unwrap();
        let records = c["records"].as_array().unwrap();
        program.careers = Some(sims_vm::program::CareerCatalog {
            ids: records
                .iter()
                .map(|r| r["id"].as_i64().unwrap() as i32)
                .collect(),
            records: Some(
                records
                    .iter()
                    .map(|r| sims_formats::career::Career {
                        version: 0,
                        packed: false,
                        name: "test".into(),
                        consumed: 0,
                        levels: r["levels"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|l| {
                                let l: [String; 4] = serde_json::from_value(l.clone()).unwrap();
                                sims_formats::career::Level {
                                    values: [0; 21],
                                    strings: [
                                        String::new(),
                                        l[0].clone(),
                                        l[1].clone(),
                                        l[2].clone(),
                                        l[3].clone(),
                                    ],
                                }
                            })
                            .collect(),
                    })
                    .collect(),
            ),
            ..Default::default()
        });
        let mut cache = c.get("cache").map(|v| career_appearance::State {
            key: serde_json::from_value(v["key"].clone()).unwrap(),
            body: v["body"].as_str().unwrap().as_bytes().to_vec(),
            texture: v["texture"].as_str().unwrap().as_bytes().to_vec(),
            extra: v["extra"].as_str().unwrap().as_bytes().to_vec(),
        });
        let names = |key| {
            c[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().as_bytes().to_vec())
                .collect()
        };
        let mut provider = Catalog {
            names: Names {
                bodies: names("bodies"),
                textures: sims_sim::pose::clothing::texture_names::Cache {
                    names: names("textures"),
                },
            },
            calls: 0,
        };
        let fields = ["career", "level", "age", "species"].map(|k| c[k].as_i64().unwrap() as i16);
        let fingerprint = program.fingerprint();
        let selected = sims_content::clothing_profile::select_career(
            &program,
            &mut profiles,
            &d,
            fields,
            &mut cache,
            &mut provider,
        )?;
        let expected = &row["original"];
        assert_eq!(
            json!([selected[0].name, selected[0].texture, selected[1].name]),
            json!([expected["body"], expected["texture"], expected["extra"]]),
            "{c}"
        );
        assert!(selected[1].texture.is_empty());
        let state = cache.as_ref().unwrap();
        assert_eq!(
            json!({"key":state.key,"body":String::from_utf8(state.body.clone())?,"texture":String::from_utf8(state.texture.clone())?,"extra":String::from_utf8(state.extra.clone())?}),
            expected["cache"],
            "{c}"
        );
        assert_eq!(json!(profiles.get(&d).unwrap()), expected["profile"], "{c}");
        let expected_calls = usize::from(
            expected["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e[0] == "bind"),
        );
        assert_eq!(provider.calls, expected_calls, "{c}");
        profiles.validate(&program).unwrap();
        let mut cold: Option<career_appearance::State> =
            serde_json::from_slice(&serde_json::to_vec(&cache)?)?;
        // Neither a changed age/gender/profile nor an unavailable installation
        // invalidates this native career/level cache on its own.
        let mut cold_profiles: State = serde_json::from_slice(&serde_json::to_vec(&profiles)?)?;
        let mut changed = fields;
        changed[2] = 17;
        changed[3] ^= 1;
        let cached = sims_content::clothing_profile::select_career(
            &Program::default(),
            &mut cold_profiles,
            &d,
            changed,
            &mut cold,
            &mut sims_content::clothing_catalog::Unavailable,
        )?;
        assert_eq!(cached, selected);
        assert_eq!(cold, cache);
        assert_eq!(cold_profiles, profiles);
        let mut person = (0..256)
            .map(|n| (n, 0))
            .collect::<std::collections::BTreeMap<i16, i16>>();
        person.insert(8, 3);
        for (key, value) in [56, 57, 58, 65].into_iter().zip(fields) {
            person.insert(key, value);
        }
        let mut selection = sims_sim::pose::appearance_selection::State::constructed([0; 3]);
        let result = sims_content::clothing_profile::select_appearance_with_career(
            &program,
            &mut cold_profiles,
            &d,
            &mut selection,
            &mut person,
            0,
            false,
            &mut cold,
            &mut sims_content::clothing_catalog::Unavailable,
        )?;
        assert!(
            !result.clothing_used,
            "cache-only selection fabricated a process catalog dependency"
        );
        assert_eq!(program.fingerprint(), fingerprint);
    }
    Ok(())
}

#[test]
fn failed_uniform_catalog_does_not_commit_selector_or_person_cache() -> anyhow::Result<()> {
    let mut p = Program::default();
    let d = Definition {
        source: "test".into(),
        object: 128,
        guid: 1,
        slots: 0,
    };
    let m = p.modules.entry(d.source.clone()).or_default();
    let mut words = vec![0; 20];
    words[9] = 2;
    words[19] = 200;
    m.object_definition_words.insert(128, words);
    m.strings.insert(200, vec![String::new(); 35]);
    p.careers = Some(sims_vm::program::CareerCatalog {
        ids: vec![1],
        records: Some(vec![sims_formats::career::Career {
            version: 0,
            packed: false,
            name: String::new(),
            consumed: 0,
            levels: vec![sims_formats::career::Level {
                values: [0; 21],
                strings: Default::default(),
            }],
        }]),
        ..Default::default()
    });
    let mut profiles = State::default();
    profiles.load(&p, &d).unwrap();
    let mut person = (0..256)
        .map(|n| (n, 0))
        .collect::<std::collections::BTreeMap<i16, i16>>();
    person.insert(8, 3);
    person.insert(56, 1);
    person.insert(58, 27);
    let mut selection = sims_sim::pose::appearance_selection::State::constructed([0; 3]);
    let mut cache = None;
    let before = (
        profiles.clone(),
        person.clone(),
        selection.clone(),
        cache.clone(),
    );
    for _ in 0..2 {
        assert!(
            sims_content::clothing_profile::select_appearance_with_career(
                &p,
                &mut profiles,
                &d,
                &mut selection,
                &mut person,
                0,
                false,
                &mut cache,
                &mut sims_content::clothing_catalog::Unavailable
            )
            .is_err()
        );
        assert_eq!(
            (
                profiles.clone(),
                person.clone(),
                selection.clone(),
                cache.clone()
            ),
            before
        );
    }
    Ok(())
}
