use serde_json::{Value, json};
use sims_vm::image_cache::{
    Entry, Event, State,
    speech::{Request, Response, Speech},
};

fn state(cached: Option<u32>, dirty: bool, slot: usize) -> State {
    let mut state = State {
        entries: vec![Entry::new(11)],
        refs: vec![1; 6],
        ui: None,
    };
    state.entries[0].dirty = [true; 7];
    state.entries[0].dirty[slot] = dirty;
    if let Some(id) = cached {
        state.entries[0].images[slot] = Some(id);
        state.refs[id as usize - 1] += 1;
    }
    state.refs[5] += 1; // transferred caller output wrapper, plus external owner
    state
}

fn supply(cache: &mut State, slot: usize, image: u32) {
    // Supplied regeneration effects, outside the compared getter. This mirrors
    // the native probe's host contract, not native regeneration itself.
    if let Some(old) = cache.entries[0].images[slot] {
        cache.refs[old as usize - 1] -= 1;
    }
    cache.refs[image as usize - 1] += 1;
    cache.entries[0].images[slot] = Some(image);
    cache.entries[0].dirty[slot] = false;
}

fn execute(c: &Value, mut cache: State, restore: bool) -> Value {
    let index = c["index"].as_i64().unwrap() as u32;
    let zoom = c["zoom"].as_i64().unwrap() as i32;
    let output: Option<u32> = serde_json::from_value(c["out"].clone()).unwrap();
    let mut run = Speech::new(index, zoom, output, c["allow"].as_bool().unwrap());
    let mut events = vec![];
    let mut releases = vec![];
    let mut destroyed = vec![];
    let mut observer = |event, s: &State| match event {
        Event::Release(image) => releases.push(json!({"image":image,"state":s})),
        Event::Destroy(image) => destroyed.push(image),
        Event::Ui => panic!("speech retrieval invalidated UI"),
    };
    while let Some(request) = run.advance(&mut cache, &mut observer).unwrap() {
        let bytes = serde_json::to_vec(&(&run, &cache)).unwrap();
        assert_eq!(
            run.advance(&mut cache, &mut observer).unwrap(),
            Some(request)
        );
        assert_eq!(bytes, serde_json::to_vec(&(&run, &cache)).unwrap());
        if restore {
            (run, cache) = serde_json::from_slice(&bytes).unwrap();
        }
        let reply = match request {
            Request::Refresh { index: i, persist } => {
                assert_eq!(i, index);
                events.push(json!({"kind":"refresh","persist":persist,"state":cache}));
                match c[if persist { "persist" } else { "refresh" }]
                    .as_u64()
                    .unwrap()
                {
                    0 => {}
                    1 => {
                        supply(&mut cache, 4, 3);
                        supply(&mut cache, 5, 4);
                    }
                    2 => supply(&mut cache, 4, 3),
                    _ => unreachable!(),
                }
                Response::Refreshed
            }
            Request::Load { index: i, chunk } => {
                assert_eq!(i, index);
                assert_eq!(chunk, if zoom == 2 { 2005 } else { 2006 });
                events.push(json!({"kind":"load","chunk":chunk,"state":cache}));
                Response::Image(serde_json::from_value(c["saved"].clone()).unwrap())
            }
        };
        let before = (run.clone(), cache.clone());
        let wrong = match reply {
            Response::Refreshed => Response::Image(None),
            Response::Image(_) => Response::Refreshed,
        };
        assert!(run.respond(&mut cache, wrong, &mut observer).is_err());
        assert_eq!(before, (run.clone(), cache.clone()));
        run.respond(&mut cache, reply, &mut observer).unwrap();
        if restore {
            (run, cache) =
                serde_json::from_slice(&serde_json::to_vec(&(&run, &cache)).unwrap()).unwrap();
        }
        run.validate(&cache).unwrap();
    }
    run.validate(&cache).unwrap();
    let (returned, out) = run.finish().unwrap();
    json!({"state":cache,"returned":returned,"out":out,"events":events,"destroyed":destroyed,"releases":releases})
}

#[test]
fn small_and_large_image_lookup_preserves_failure_order_and_restores() {
    let mut cases = 0;
    for zoom in [2, 3] {
        for cached in [None, Some(1)] {
            for dirty in [false, true] {
                for allow in [false, true] {
                    for saved in [None, Some(2)] {
                        for refresh in [0, 1, 2] {
                            for persist in [0, 1, 2] {
                                let c = json!({"index":0,"zoom":zoom,"out":6,"allow":allow,"saved":saved,"refresh":refresh,"persist":persist});
                                let initial = state(cached, dirty, if zoom == 2 { 4 } else { 5 });
                                let out = execute(&c, initial.clone(), false);
                                assert_eq!(out, execute(&c, initial.clone(), true));
                                let expected = if dirty && !allow {
                                    None
                                } else {
                                    let regenerated = if dirty && refresh == 1
                                        || dirty && refresh == 2 && zoom == 2
                                    {
                                        Some(if zoom == 2 { 3 } else { 4 })
                                    } else {
                                        cached
                                    };
                                    regenerated.or(saved).or_else(|| {
                                        (allow && (persist == 1 || persist == 2 && zoom == 2))
                                            .then_some(if zoom == 2 { 3 } else { 4 })
                                    })
                                };
                                assert_eq!(out["returned"], expected.is_some(), "{c}");
                                assert_eq!(
                                    out["out"],
                                    if dirty && !allow {
                                        json!(6)
                                    } else {
                                        json!(expected)
                                    },
                                    "{c}"
                                );
                                if dirty && !allow {
                                    assert_eq!(out["state"], json!(initial));
                                    assert!(out["events"].as_array().unwrap().is_empty());
                                    assert!(out["releases"].as_array().unwrap().is_empty());
                                }
                                for slot in [0, 1, 2, 3, 6] {
                                    assert_eq!(
                                        out["state"]["entries"][0]["images"][slot],
                                        Value::Null
                                    );
                                    assert_eq!(out["state"]["entries"][0]["dirty"][slot], true);
                                }
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 288);
}

#[test]
fn invalid_selection_preserves_the_existing_wrapper() {
    for zoom in [-1, 0, 1, 2, 3, 4] {
        for index in [-1, 0, 1] {
            if index == 0 && (zoom == 2 || zoom == 3) {
                continue;
            }
            let c = json!({"index":index,"zoom":zoom,"out":6,"allow":true,"saved":2,"refresh":1,"persist":1});
            let initial = state(Some(1), true, 4);
            let out = execute(&c, initial.clone(), true);
            assert_eq!(out["returned"], false);
            assert_eq!(out["out"], 6);
            assert_eq!(out["state"], json!(initial));
            assert_eq!(out["events"], json!([]));
        }
    }
}

#[test]
fn serialized_request_rejects_wrong_chunk_and_lost_output_owner() {
    let mut cache = state(None, false, 4);
    let mut run = Speech::new(0, 2, Some(6), false);
    assert_eq!(
        run.advance(&mut cache, &mut |_, _| {}).unwrap(),
        Some(Request::Load {
            index: 0,
            chunk: 2005
        })
    );
    let mut value = serde_json::to_value(&run).unwrap();
    value["pending"]["Load"]["chunk"] = json!(2006);
    assert!(
        serde_json::from_value::<Speech>(value)
            .unwrap()
            .validate(&cache)
            .is_err()
    );
    run.respond(&mut cache, Response::Image(Some(2)), &mut |_, _| {})
        .unwrap();
    cache.refs[1] = 0;
    assert!(run.validate(&cache).is_err());
}

#[test]
#[ignore = "requires guarded private original 4e915d execution report"]
fn speech_retrieval_matches_original_image_ownership_and_callbacks() {
    let path = std::env::var("SIMS_SPEECH_IMAGE_REPORT").expect("private native report path");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let original = &row["original"];
        let initial: State = serde_json::from_value(original["initial"].clone()).unwrap();
        let mut expected = original.clone();
        expected.as_object_mut().unwrap().remove("initial");
        for restore in [false, true] {
            assert_eq!(
                execute(&row["input"], initial.clone(), restore),
                expected,
                "{}",
                row["input"]
            );
        }
    }
}
