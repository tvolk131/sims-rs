use serde_json::{Value, json};
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    appearance_selection::{self, Request, Selection, State},
};

#[test]
fn unsupported_profile_selection_cannot_poison_a_retry_cache() {
    let mut profile = vec![String::new(); 40];
    profile[2] = "head,HEAD=texture".into();
    let mut person: std::collections::BTreeMap<i16, i16> = (0..256).map(|n| (n, 0)).collect();
    person.insert(8, 5); // Empty entry 33 requires the unresolved pajama helper.
    let mut state = State::constructed([255; 3]);
    let before = (state.clone(), person.clone());
    for _ in 0..2 {
        assert!(Selection::begin(&mut state, &mut person, &profile, 0, false).is_err());
        assert_eq!((state.clone(), person.clone()), before);
    }
    // Missing descriptors after a represented outfit must be retryable too.
    person.insert(8, 0);
    let before = (state.clone(), person.clone());
    assert!(Selection::begin(&mut state, &mut person, &profile[..3], 0, false).is_err());
    assert_eq!((state, person), before);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_profile_descriptors_preserve_matching_keys_and_buffer_limits() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-appearance-selection.json")).unwrap();
    for row in report["descriptors"].as_array().unwrap() {
        let c = &row["input"];
        let d = appearance_selection::descriptor(
            c["text"].as_str().unwrap(),
            c["secondary"].as_bool().unwrap(),
        );
        assert_eq!(json!([d.name, d.texture]), row["original"], "{c}");
    }
    for c in report["constructor_caches"].as_array().unwrap() {
        assert_eq!(json!(State::constructed([0; 3]).cached), c["cache"]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_profile_appearance_selection_order_caches_and_prefix_comparisons() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-appearance-selection.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 170);
    let pajamas: Value = serde_json::from_str(include_str!(
        "fixtures/native-appearance-pajama-branch.json"
    ))
    .unwrap();
    assert_eq!(pajamas["cases"].as_array().unwrap().len(), 96);
    let careers: Value = serde_json::from_str(include_str!(
        "fixtures/native-appearance-career-branch.json"
    ))
    .unwrap();
    assert_eq!(careers["cases"].as_array().unwrap().len(), 16);
    for row in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(pajamas["cases"].as_array().unwrap())
        .chain(careers["cases"].as_array().unwrap())
    {
        let c = &row["input"];
        let profile: Vec<String> = serde_json::from_value(c["profile"].clone()).unwrap();
        let mut person = (0..256)
            .map(|n| (n, 0))
            .collect::<std::collections::BTreeMap<i16, i16>>();
        for (n, v) in c["person"].as_object().unwrap() {
            person.insert(n.parse().unwrap(), v.as_i64().unwrap() as i16);
        }
        let mut state = State::constructed([23, 47, 91]);
        state.cached = if c["cache"].is_array() {
            serde_json::from_value(c["cache"].clone()).unwrap()
        } else {
            [-1, -1, 0, 0]
        };
        let mut records = Records::default();
        if let Some(entries) = c["records"].as_array() {
            for (i, r) in entries.iter().enumerate() {
                records.entries[i] = Record {
                    binding: r["bound"].as_bool().unwrap_or(false).then_some(Binding {
                        id: i as u64 + 1,
                        owner: 1,
                    }),
                    name: r["name"].as_str().unwrap_or("").into(),
                    texture: r["texture"].as_str().unwrap_or("").into(),
                };
            }
        }
        let mut called = None;
        let mut career_called = false;
        let mut execution = Selection::begin_with_helpers(
            &mut state,
            &mut person,
            &profile,
            c["flags"].as_u64().unwrap_or(0) as u16,
            c["force"].as_bool().unwrap_or(false),
            |helper| {
                if helper == appearance_selection::Helper::Career {
                    career_called = true;
                    return Ok([
                        appearance_selection::Descriptor {
                            name: c["career"]["body"].as_str().unwrap().into(),
                            texture: c["career"]["texture"].as_str().unwrap().into(),
                        },
                        appearance_selection::Descriptor {
                            name: c["career"]["extra"].as_str().unwrap().into(),
                            texture: String::new(),
                        },
                    ]);
                }
                let appearance_selection::Helper::Pajamas(age) = helper else {
                    unreachable!()
                };
                assert!(c["pajamas"].is_object());
                called = Some(age);
                Ok([
                    appearance_selection::Descriptor {
                        name: c["pajamas"]["body"].as_str().unwrap().into(),
                        texture: c["pajamas"]["texture"].as_str().unwrap().into(),
                    },
                    Default::default(),
                ])
            },
        )
        .unwrap();
        let mut events: Vec<Value> = execution
            .lookups
            .iter()
            .flat_map(|entry| {
                let mut result = vec![];
                if *entry == 2 && career_called {
                    result.push(json!({"career":true}));
                    career_called = false;
                }
                result.push(json!({"profile":entry}));
                if *entry == 33
                    && let Some(age) = called.take()
                {
                    result.push(json!({"pajamas":age}));
                }
                result
            })
            .collect();
        if career_called {
            events.push(json!({"career":true}));
        }
        while let Some(request) = execution.next(&state, &records) {
            let snapshot = execution.clone();
            execution = serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
            assert_eq!(execution, snapshot);
            assert_eq!(execution.next(&state, &records), Some(request.clone()));
            assert_eq!(execution, snapshot);
            match request {
                Request::Bind {record,descriptor} => {
                    events.push(json!({"bind":{"record":record,"name":descriptor.name,"texture":descriptor.texture}}));
                    if !descriptor.name.is_empty() {
                        records.entries[record]=Record {binding:Some(Binding{id:record as u64+1,owner:1}),name:descriptor.name,texture:descriptor.texture};
                    }
                }
                Request::TextureNotification {first} => events.push(json!({"texture_notification":if first {"4eb0cc"} else {"4eb0eb"},"guid":0x12345678u32})),
                Request::Tint(tint) => events.push(json!({"tint":tint})),
            }
            execution.acknowledge().unwrap();
        }
        assert!(execution.is_complete());
        assert_eq!(json!(events), row["events"], "{c}");
        assert_eq!(json!(state.cached), row["original"]["cache"]);
        assert_eq!(json!(state.dirty), row["original"]["dirty"]);
        assert_eq!(json!(state.special_dirty), row["original"]["special_dirty"]);
        assert_eq!(json!(state.skin_dirty), row["original"]["skin_dirty"]);
        assert_eq!(
            json!(person.values().copied().collect::<Vec<_>>()),
            row["original"]["person"]
        );
        assert_eq!(
            json!(
                records
                    .entries
                    .iter()
                    .map(|r| json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture}))
                    .collect::<Vec<_>>()
            ),
            row["original"]["records"]
        );
    }
}
