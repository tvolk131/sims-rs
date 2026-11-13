use super::*;
use serde_json::{Value, json};

fn source() -> Binding {
    Binding {
        module: "Global.iff".into(),
        module_sha256: "bdfd8456abceaa3e44f0a9754815248ea1e80c7b0df4fbaff53843fae3731478".into(),
        resource_id: 2,
        resource_sha256: "fd3a44e2068aa43ba01d17d79bafb2cd8882beb44a2f53376d34f6172c637a2e".into(),
    }
}
fn configuration() -> Configuration {
    Configuration {
        profile: Some(Profile::CompleteCollection191),
        source: Some(source()),
        fp_control: Some(0x7f),
    }
}
fn cold(call: &mut Call, process: &mut Process) {
    let bytes = serde_json::to_vec(&(&*call, &*process)).unwrap();
    let (c, p): (Call, Process) = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&(&c, &p)).unwrap(), bytes);
    c.validate().unwrap();
    *call = c;
    *process = p;
}
fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/native-autonomy-constants.json"
    ))
    .unwrap()
}
fn store_json(store: Store) -> Value {
    match store {
        Store::Word(field, value) => json!({"address":field.address(),"size":4,"value":value}),
        Store::Loaded(value) => json!({"address":0x6e5291,"size":1,"value":value}),
    }
}

#[test]
fn original48_plus_name_default_and_scoped_float_vectors_match_every_store_and_cold_cut() {
    let corpus = fixture();
    assert_eq!(
        corpus["sealed48_sha256"],
        "a7e87d99827e7dafc0891bf7cfcb943d365aacd4fa4c4daa89944a850843ed95"
    );
    assert_eq!(
        corpus["playable_sha256"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 61);
    check_original_cases(cases, 59, 2);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn actual_initial_003f_all_supplied_vectors_match_each_store_table_and_cold_cut() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/native-autonomy-003f.json"
    ))
    .unwrap();
    assert_eq!(
        corpus["original_gate_observation_sha256"],
        "e5a121e3f616fd8c96723eae37e23f2b82a0c20f50bf247780da5449bd8b8e1f"
    );
    assert_eq!(
        corpus["raw_report_sha256"],
        "8b319958f456138d32c4d5778323a2d3210f0a9ba27bedf14982e0ef5b352155"
    );
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 82);
    assert!(cases.iter().all(|case| case["fp_control"] == 0x3f));
    check_original_cases(cases, 80, 2);
}

fn check_original_cases(cases: &[Value], expected_supported: usize, expected_unsupported: usize) {
    let vm = crate::Vm::new(crate::World::default(), 0x10203040);
    let vm_before = serde_json::to_vec(&vm).unwrap();
    let mut supported = 0;
    let mut deliberately_unsupported = 0;
    for case in cases {
        let label = &case["label"];
        let mut process: Process = serde_json::from_value(case["before"].clone()).unwrap();
        let mut config = configuration();
        config.fp_control = Some(case["fp_control"].as_u64().unwrap() as u16);
        let mut call = Call::new(config);
        let mut writes = vec![];
        let mut stopped = false;
        let mut completed = false;
        for _ in 0..32 {
            cold(&mut call, &mut process);
            let progress = call.advance(&mut process);
            if case["unsupported_signaling_float"] == true {
                match &progress {
                    Err(e) => {
                        assert_eq!(
                            e, "autonomy constants signaling-NaN float transfer is unverified",
                            "{label}"
                        );
                        assert!(writes.is_empty());
                        assert_eq!(
                            serde_json::to_value(&process).unwrap(),
                            case["before"],
                            "{label}"
                        );
                        cold(&mut call, &mut process);
                        deliberately_unsupported += 1;
                        stopped = true;
                        break;
                    }
                    Ok(Progress::Service(_)) => {}
                    other => panic!("{label}: unexpected signaling float step {other:?}"),
                }
            }
            match progress.unwrap() {
                Progress::Store(store) => writes.push(store_json(store)),
                Progress::Complete => {
                    completed = true;
                    break;
                }
                Progress::Service(Request::Load(binding)) => {
                    let table = Table {
                        rows: serde_json::from_value(case["source_table"].clone()).unwrap(),
                    };
                    call.reply(Reply::Loaded { binding, table }).unwrap();
                }
                Progress::Service(Request::Flush(binding)) => {
                    assert_eq!(
                        serde_json::to_value(&call.table().unwrap().rows).unwrap(),
                        case["flush_table"],
                        "{label}"
                    );
                    if case["stop_at_flush"] == true {
                        cold(&mut call, &mut process);
                        assert!(matches!(
                            call.advance(&mut process).unwrap(),
                            Progress::Service(Request::Flush(_))
                        ));
                        stopped = true;
                        break;
                    }
                    call.reply(Reply::Flushed {
                        binding,
                        return_value: case["flush_result"].as_i64().unwrap() as i32,
                    })
                    .unwrap();
                    //The flush reply itself does not move the later loaded store.
                    assert_eq!(process.loaded, Some(0));
                }
            }
        }
        if case["unsupported_signaling_float"] == true {
            continue;
        }
        supported += 1;
        assert!(
            completed || stopped,
            "{label}: bounded producer did not finish"
        );
        assert_eq!(
            writes,
            case["writes"].as_array().unwrap().as_slice(),
            "{label}"
        );
        assert_eq!(
            serde_json::to_value(&process).unwrap(),
            case["after"],
            "{label}"
        );
        cold(&mut call, &mut process);
        if !stopped {
            let before = process.clone();
            //An entirely changed/unknown provider on the next person is not read.
            let mut next = Call::new(Configuration::default());
            assert_eq!(
                next.advance(&mut process).unwrap(),
                Progress::Complete,
                "{label}"
            );
            assert_eq!(process, before, "{label}");
            cold(&mut next, &mut process);
        }
    }
    assert_eq!(supported, expected_supported);
    assert_eq!(deliberately_unsupported, expected_unsupported);
    assert_eq!(serde_json::to_vec(&vm).unwrap(), vm_before);
}

#[test]
fn original_raw_name_comparison_preserves_full_lengths_ascii_fold_and_first_duplicate() {
    let corpus = fixture();
    let cases = corpus["lookups"].as_array().unwrap();
    assert_eq!(cases.len(), 7);
    for case in cases {
        let table: Table = serde_json::from_value(case["table"].clone()).unwrap();
        let query: Vec<u8> = serde_json::from_value(case["query"].clone()).unwrap();
        let expected: Option<usize> = serde_json::from_value(case["index"].clone()).unwrap();
        assert_eq!(table.find(&query), expected, "{}", case["label"]);
    }
}

#[test]
fn loaded_unknown_and_nonzero_do_not_invent_process_or_configuration_values() {
    let mut process = Process::default();
    let mut call = Call::new(Configuration::default());
    let before = serde_json::to_vec(&(&call, &process)).unwrap();
    assert_eq!(
        call.advance(&mut process).unwrap_err(),
        "autonomy constants loaded byte is unknown"
    );
    assert_eq!(serde_json::to_vec(&(&call, &process)).unwrap(), before);
    for raw in [1, 2, 255] {
        process.loaded = Some(raw);
        let before = process.clone();
        let mut call = Call::new(Configuration {
            source: Some(Binding {
                module: String::new(),
                module_sha256: String::new(),
                resource_id: 0,
                resource_sha256: String::new(),
            }),
            ..Default::default()
        });
        assert_eq!(call.advance(&mut process).unwrap(), Progress::Complete);
        assert_eq!(process, before);
        assert!(call.table().is_none());
        assert!(process.words.iter().all(Option::is_none));
        cold(&mut call, &mut process);
    }
    let legacy: Process = serde_json::from_value(json!({"loaded":null,"words":[null,null,null,null,null,null,null,null,null,null,null,null]})).unwrap();
    assert_eq!(legacy, Process::default());
}

#[test]
fn unknown_profile_source_and_conversion_stop_in_native_order_with_prior_stores_retained() {
    let mut process = Process {
        loaded: Some(0),
        ..Default::default()
    };
    let mut call = Call::new(Configuration::default());
    assert_eq!(
        call.advance(&mut process).unwrap_err(),
        "autonomy constants code profile is unknown"
    );
    let mut config = configuration();
    config.source = None;
    let mut call = Call::new(config);
    assert_eq!(
        call.advance(&mut process).unwrap_err(),
        "autonomy constants FCNS2 source is unknown"
    );
    for control in [None, Some(0x37e)] {
        let mut process = Process {
            loaded: Some(0),
            ..Default::default()
        };
        let mut config = configuration();
        config.fp_control = control;
        let mut call = Call::new(config);
        assert_eq!(
            call.advance(&mut process).unwrap(),
            Progress::Service(Request::Load(source()))
        );
        call.reply(Reply::Loaded {
            binding: source(),
            table: Table::default(),
        })
        .unwrap();
        for field in ORDER.into_iter().take(9) {
            assert!(
                matches!(call.advance(&mut process).unwrap(),Progress::Store(Store::Word(f,_))if f==field)
            );
            cold(&mut call, &mut process);
        }
        let before = process.clone();
        let error = call.advance(&mut process).unwrap_err();
        assert_eq!(
            error,
            if control.is_none() {
                "autonomy constants x87 control is unknown"
            } else {
                "autonomy constants x87 control is unverified"
            }
        );
        assert_eq!(process, before);
        assert!(process.words[..9].iter().all(Option::is_some));
        assert!(process.words[9..].iter().all(Option::is_none));
        //Missing random-count default was inserted BEFORE the unsupported ftol.
        assert_eq!(call.table().unwrap().rows.len(), 10);
        assert_eq!(call.table().unwrap().rows[9].value_bits, 0x4120_0000);
        cold(&mut call, &mut process);
        assert_eq!(call.advance(&mut process).unwrap_err(), error);
        assert_eq!(call.table().unwrap().rows.len(), 10);
    }
}

#[test]
fn resource_reply_faults_are_transactional_and_unanswered_flush_never_sets_loaded() {
    let mut process = Process {
        loaded: Some(0),
        ..Default::default()
    };
    let mut call = Call::new(configuration());
    assert!(matches!(
        call.advance(&mut process).unwrap(),
        Progress::Service(Request::Load(_))
    ));
    let mut wrong = source();
    wrong.resource_sha256 = "0".repeat(64);
    let before = serde_json::to_vec(&(&call, &process)).unwrap();
    let malformed = Table {
        rows: vec![Row {
            name: vec![0],
            value_bits: 0,
            comment: vec![],
        }],
    };
    assert_eq!(
        call.reply(Reply::Loaded {
            binding: wrong,
            table: malformed.clone()
        })
        .unwrap_err(),
        "autonomy constants resource reply differs"
    );
    assert_eq!(serde_json::to_vec(&(&call, &process)).unwrap(), before);
    assert_eq!(
        call.reply(Reply::Loaded {
            binding: source(),
            table: malformed
        })
        .unwrap_err(),
        "autonomy constants native row string is invalid"
    );
    assert_eq!(serde_json::to_vec(&(&call, &process)).unwrap(), before);
    call.reply(Reply::Loaded {
        binding: source(),
        table: Table::default(),
    })
    .unwrap();
    for _ in 0..12 {
        assert!(matches!(
            call.advance(&mut process).unwrap(),
            Progress::Store(Store::Word(_, _))
        ));
    }
    let progress = call.advance(&mut process).unwrap();
    assert_eq!(progress, Progress::Service(Request::Flush(source())));
    let before = serde_json::to_vec(&(&call, &process)).unwrap();
    for _ in 0..3 {
        cold(&mut call, &mut process);
        assert_eq!(call.advance(&mut process).unwrap(), progress);
        assert_eq!(serde_json::to_vec(&(&call, &process)).unwrap(), before);
        assert_eq!(process.loaded, Some(0));
    }
    assert_eq!(
        call.reply(Reply::Loaded {
            binding: source(),
            table: Table::default()
        })
        .unwrap_err(),
        "autonomy constants resource reply differs"
    );
    assert_eq!(serde_json::to_vec(&(&call, &process)).unwrap(), before);
    call.reply(Reply::Flushed {
        binding: source(),
        return_value: 0,
    })
    .unwrap();
    cold(&mut call, &mut process);
    assert_eq!(process.loaded, Some(0));
    assert_eq!(
        call.advance(&mut process).unwrap(),
        Progress::Store(Store::Loaded(1))
    );
    assert_eq!(process.loaded, Some(1));
    assert!(call.table().is_some());
    cold(&mut call, &mut process);
    assert_eq!(call.advance(&mut process).unwrap(), Progress::Complete);
    assert!(call.table().is_none());
}

#[test]
fn byte_payloads_comments_unknown_words_and_malformed_continuations_are_not_normalized() {
    let table = Table {
        rows: vec![Row {
            name: vec![0xc4, b'A'],
            value_bits: 0x7fc1_2345,
            comment: vec![0x80, b'Z'],
        }],
    };
    table.validate().unwrap();
    assert_eq!(
        serde_json::from_slice::<Table>(&serde_json::to_vec(&table).unwrap()).unwrap(),
        table
    );
    for invalid in [
        Table {
            rows: vec![Row {
                name: vec![b'a'; 64],
                value_bits: 0,
                comment: vec![],
            }],
        },
        Table {
            rows: vec![Row {
                name: vec![],
                value_bits: 0,
                comment: vec![b'a'; 256],
            }],
        },
    ] {
        assert!(invalid.validate().is_err());
    }
    let mut call = Call::new(configuration());
    let mut value = serde_json::to_value(&call).unwrap();
    value["stage"] = json!("Flush");
    value["table"] = json!({"rows":[]});
    value["configuration"]["source"] = Value::Null;
    call = serde_json::from_value(value).unwrap();
    let mut process = Process {
        loaded: Some(0),
        ..Default::default()
    };
    assert!(call.advance(&mut process).is_err());
    assert!(process.words.iter().all(Option::is_none));
}
