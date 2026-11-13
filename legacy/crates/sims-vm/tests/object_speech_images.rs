use serde_json::{Value, json};
use sims_vm::image_cache::{
    Event, State,
    object_speech::{Definition, Graphic, Request, Response, Retrieval},
};

fn execute_probe_case(c: &Value, mut cache: State, mut graphic: Graphic, restore: bool) -> Value {
    let mut operation = Retrieval::new(
        c["zoom"].as_i64().unwrap() as i32,
        serde_json::from_value(c["out"].clone()).unwrap(),
    );
    let mut force = c["force"].as_bool().unwrap();
    let mut events = vec![];
    let mut releases = vec![];
    let mut destroyed = vec![];
    let mut observer = |event, state: &State| match event {
        Event::Release(image) => releases.push(json!({"image":image,"state":state})),
        Event::Destroy(image) => destroyed.push(image),
        Event::Ui => panic!("object speech invalidated person UI"),
    };
    let mut loads = 0;
    while let Some(request) = operation
        .advance(&mut cache, &mut graphic, force, &mut observer)
        .unwrap()
    {
        if restore {
            roundtrip(&mut cache, &mut graphic, &mut operation);
        }
        // Definition access is native in-memory data, not an intercepted service.
        if !matches!(request, Request::Definition { .. }) {
            let tagged = serde_json::to_value(request).unwrap();
            let (kind, fields) = tagged.as_object().unwrap().iter().next().unwrap();
            let mut event = fields.as_object().unwrap().clone();
            event.insert("kind".into(), json!(kind.to_lowercase()));
            event.insert("state".into(), json!(cache));
            event.insert("graphic".into(), json!(graphic));
            events.push(Value::Object(event));
        }
        let response = match request {
            Request::Definition { .. } => Response::Definition(Definition {
                group: c["group"].as_i64().unwrap() as i16,
                base_graphic: c["base_graphic"].as_i64().unwrap() as i16,
                icon_base: c["icon_base"].as_i64().unwrap() as i16,
            }),
            Request::Load { chunk, .. } => {
                assert_eq!(
                    chunk,
                    c["icon_base"].as_i64().unwrap() as i32 + (loads + 1) * 2000
                );
                let image = c[if loads == 0 {
                    "saved_small"
                } else {
                    "saved_large"
                }]
                .as_bool()
                .unwrap()
                .then_some(loads as u32 + 3);
                loads += 1;
                if loads == 2 {
                    if let Some(value) = c["after_load_force"].as_bool() {
                        force = value;
                    }
                }
                Response::Image(image)
            }
            Request::Template { chunk } => Response::Image(
                c[if chunk == 9000 {
                    "template_small"
                } else {
                    "template_large"
                }]
                .as_bool()
                .unwrap()
                .then_some(u32::from(chunk - 9000) + 5),
            ),
            Request::Generate { .. } => {
                // Effects supplied by the same explicit host boundary in the probe.
                if c["clear_small"].as_bool().unwrap() {
                    if let Some(old) = graphic.images[0].take() {
                        cache.refs[old as usize - 1] -= 1;
                    }
                }
                graphic.resource = Some(c["generated_resource"].as_u64().unwrap() as u32);
                Response::Done
            }
            Request::Encode { image } => Response::Buffer(
                (c["lock"].as_bool().unwrap() && c["encode"].as_bool().unwrap())
                    .then_some(image + 100),
            ),
            _ => Response::Done,
        };
        operation
            .respond(&mut cache, &mut graphic, response, &mut observer)
            .unwrap();
        if restore {
            roundtrip(&mut cache, &mut graphic, &mut operation);
        }
    }
    let (returned, out) = operation.finish().unwrap();
    json!({"state":cache,"graphic":graphic,"out":out,"returned":returned,
        "events":events,"releases":releases,"destroyed":destroyed})
}

#[test]
#[ignore = "requires stdlib-only private probe input matrix"]
fn prepared_object_speech_matrix_restores_at_every_service_boundary() {
    let path = std::env::var("SIMS_OBJECT_SPEECH_INPUTS").expect("prepared matrix path");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["scope"],
        "synthetic inputs only; no native execution"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 368);
    for c in report["cases"].as_array().unwrap() {
        let (mut cache, mut graphic) =
            setup(c["small"].as_bool().unwrap(), c["large"].as_bool().unwrap());
        graphic.kind = c["kind"].as_i64().unwrap() as i32;
        graphic.resource = serde_json::from_value(c["resource"].clone()).unwrap();
        cache.refs[6] -= 1; // setup's caller output is replaced by this case's owner.
        if let Some(out) = c["out"].as_u64() {
            cache.refs[out as usize - 1] += 1;
            if c["sole_out"].as_bool().unwrap() {
                cache.refs[out as usize - 1] -= 1;
            }
        }
        assert_eq!(
            execute_probe_case(c, cache.clone(), graphic.clone(), false),
            execute_probe_case(c, cache, graphic, true),
            "{c}"
        );
    }
}

#[test]
#[ignore = "requires guarded private original 4cc6a3/4cc3b9 execution report"]
fn object_speech_matches_original_ownership_and_callbacks() {
    let path = std::env::var("SIMS_OBJECT_SPEECH_REPORT").expect("private native report path");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 368);
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let initial: State = serde_json::from_value(row["original"]["initial"].clone()).unwrap();
        let graphic: Graphic =
            serde_json::from_value(row["original"]["initial_graphic"].clone()).unwrap();
        let mut expected = row["original"].clone();
        expected.as_object_mut().unwrap().remove("initial");
        expected.as_object_mut().unwrap().remove("initial_graphic");
        for restore in [false, true] {
            assert_eq!(
                execute_probe_case(&row["input"], initial.clone(), graphic.clone(), restore),
                expected,
                "{}",
                row["input"]
            );
        }
    }
}

fn setup(small: bool, large: bool) -> (State, Graphic) {
    let mut cache = State {
        refs: vec![1; 8],
        ui: None,
        ..Default::default()
    };
    let images = [small.then_some(1), large.then_some(2)];
    for id in images.into_iter().flatten().chain([7]) {
        cache.refs[id as usize - 1] += 1;
    }
    (
        cache,
        Graphic {
            kind: 1,
            resource: Some(1),
            images,
        },
    )
}

fn roundtrip(cache: &mut State, graphic: &mut Graphic, operation: &mut Retrieval) {
    (*cache, *graphic, *operation) =
        serde_json::from_slice(&serde_json::to_vec(&(&cache, &graphic, &operation)).unwrap())
            .unwrap();
    operation.validate(cache, graphic).unwrap();
}

#[test]
fn object_speech_cached_small_gate_and_saved_loads_restore() {
    for zoom in [0, 1, 2, 3, 4] {
        for small in [false, true] {
            for large in [false, true] {
                let mut pair = vec![];
                for restoring in [false, true] {
                    let (mut cache, mut graphic) = setup(small, large);
                    let before = (cache.clone(), graphic.clone());
                    let mut op = Retrieval::new(zoom, Some(7));
                    let mut requests = vec![];
                    while let Some(request) = op
                        .advance(&mut cache, &mut graphic, true, &mut |_, _| {})
                        .unwrap()
                    {
                        requests.push(request);
                        // The outer getter does not consult force when SMALL exists.
                        assert!(!small && (zoom == 2 || zoom == 3));
                        let before = (cache.clone(), graphic.clone(), op.clone());
                        assert_eq!(
                            op.advance(&mut cache, &mut graphic, false, &mut |_, _| {})
                                .unwrap(),
                            Some(request)
                        );
                        assert_eq!(before, (cache.clone(), graphic.clone(), op.clone()));
                        if restoring {
                            roundtrip(&mut cache, &mut graphic, &mut op);
                        }
                        let reply = match request {
                            Request::Definition { resource: 1 } => {
                                Response::Definition(Definition {
                                    group: 1,
                                    base_graphic: 0,
                                    icon_base: 2002,
                                })
                            }
                            Request::Load {
                                resource: 1,
                                chunk: 4002,
                            } => Response::Image(Some(3)),
                            Request::Load {
                                resource: 1,
                                chunk: 6002,
                            } => Response::Image(Some(4)),
                            Request::Template { chunk: 9000 } => Response::Image(Some(5)),
                            Request::Template { chunk: 9001 } => Response::Image(Some(6)),
                            Request::Generate {
                                resource: 1,
                                images: [5, 6],
                            } => Response::Done,
                            Request::Encode { .. } => Response::Buffer(None),
                            _ => panic!("unexpected request {request:?}"),
                        };
                        op.respond(&mut cache, &mut graphic, reply, &mut |_, _| {})
                            .unwrap();
                        if restoring {
                            roundtrip(&mut cache, &mut graphic, &mut op);
                        }
                    }
                    let (returned, output) = op.finish().unwrap();
                    if !(2..=3).contains(&zoom) {
                        assert!(!returned);
                        assert_eq!(output, Some(7));
                        assert_eq!((cache.clone(), graphic.clone()), before);
                    } else if small {
                        assert_eq!(output, graphic.images[(zoom - 2) as usize]);
                        assert_eq!(returned, output.is_some());
                        assert!(
                            requests.is_empty(),
                            "large missing must not trigger a load when small exists"
                        );
                    } else {
                        assert!(returned);
                        assert_eq!(output, Some(if zoom == 2 { 5 } else { 6 }));
                    }
                    pair.push((cache, graphic, requests, returned, output));
                }
                assert_eq!(pair[0], pair[1]);
            }
        }
    }
}

#[test]
fn loading_retains_old_large_on_failed_read_and_uses_signed_chunk_base() {
    let (mut cache, mut graphic) = setup(false, true);
    let mut op = Retrieval::new(3, Some(7));
    let mut seen = vec![];
    while let Some(request) = op
        .advance(&mut cache, &mut graphic, false, &mut |_, _| {})
        .unwrap()
    {
        seen.push(request);
        let reply = match request {
            Request::Definition { .. } => Response::Definition(Definition {
                group: 0,
                base_graphic: 1,
                icon_base: -32768,
            }),
            Request::Load { chunk: -30768, .. } => Response::Image(Some(3)),
            Request::Load { chunk: -28768, .. } => Response::Image(None),
            other => panic!("old large should survive; unexpected {other:?}"),
        };
        op.respond(&mut cache, &mut graphic, reply, &mut |_, _| {})
            .unwrap();
    }
    assert_eq!(seen.len(), 3);
    assert_eq!(graphic.images, [Some(3), Some(2)]);
    assert_eq!(op.finish().unwrap(), (true, Some(2)));
}

#[test]
fn partial_generation_persists_in_native_order_and_ignores_loader_failure() {
    for small_template in [false, true] {
        for large_template in [false, true] {
            for clear_small in [false, true] {
                for encode in [false, true] {
                    let mut pair = vec![];
                    for restoring in [false, true] {
                        let (mut cache, mut graphic) = setup(false, false);
                        let mut op = Retrieval::new(3, Some(7));
                        let mut requests = vec![];
                        let mut releases = vec![];
                        while let Some(request) = op
                            .advance(&mut cache, &mut graphic, false, &mut |e, _| {
                                releases.push(format!("{e:?}"))
                            })
                            .unwrap()
                        {
                            requests.push(request);
                            if restoring {
                                roundtrip(&mut cache, &mut graphic, &mut op);
                            }
                            let wrong = if matches!(
                                request,
                                Request::Definition { .. }
                                    | Request::Load { .. }
                                    | Request::Template { .. }
                                    | Request::Encode { .. }
                            ) {
                                Response::Done
                            } else {
                                Response::Image(None)
                            };
                            let before = (cache.clone(), graphic.clone(), op.clone());
                            assert!(
                                op.respond(&mut cache, &mut graphic, wrong, &mut |_, _| {})
                                    .is_err()
                            );
                            assert_eq!(before, (cache.clone(), graphic.clone(), op.clone()));
                            let reply = match request {
                                Request::Definition { .. } => Response::Definition(Definition {
                                    group: 1,
                                    base_graphic: 0,
                                    icon_base: 12,
                                }),
                                Request::Load { .. } => Response::Image(None),
                                Request::Template { chunk: 9000 } => {
                                    Response::Image(small_template.then_some(5))
                                }
                                Request::Template { chunk: 9001 } => {
                                    Response::Image(large_template.then_some(6))
                                }
                                Request::Generate { .. } => {
                                    if clear_small {
                                        cache
                                            .assign_image(
                                                &mut graphic.images[0],
                                                None,
                                                &mut |_, _| {},
                                            )
                                            .unwrap();
                                    }
                                    // Native rereads the descriptor's resource for persistence.
                                    graphic.resource = Some(2);
                                    Response::Done
                                }
                                Request::Encode { image } => {
                                    Response::Buffer(encode.then_some(image + 100))
                                }
                                _ => Response::Done,
                            };
                            op.respond(&mut cache, &mut graphic, reply, &mut |e, _| {
                                releases.push(format!("{e:?}"))
                            })
                            .unwrap();
                            if restoring {
                                roundtrip(&mut cache, &mut graphic, &mut op);
                            }
                        }
                        let resource = graphic.resource.unwrap();
                        for (slot, image) in graphic.images.into_iter().enumerate() {
                            if let Some(image) = image {
                                let start = requests
                                    .iter()
                                    .position(|r| *r == Request::Encode { image })
                                    .unwrap();
                                if encode {
                                    assert_eq!(
                                        &requests[start + 1..start + 7],
                                        &[
                                            Request::Add {
                                                resource,
                                                chunk: 12 + (slot as i32 + 1) * 2000,
                                                buffer: image + 100
                                            },
                                            Request::Error { resource },
                                            Request::Flush { resource },
                                            Request::Error { resource },
                                            Request::Release {
                                                resource,
                                                buffer: image + 100
                                            },
                                            Request::Error { resource },
                                        ]
                                    );
                                }
                            }
                        }
                        let returned = op.finish().unwrap();
                        assert_eq!(
                            returned,
                            (large_template, large_template.then_some(6)),
                            "getter ignores the loader's both-images result"
                        );
                        pair.push((cache, graphic, requests, releases, returned));
                    }
                    assert_eq!(pair[0], pair[1]);
                }
            }
        }
    }
}

#[test]
fn resource_gate_keeps_cached_wrappers_and_rejects_lost_ownership() {
    for kind in [0, 1, 2] {
        for resource in [None, Some(1)] {
            let (mut cache, mut graphic) = setup(false, true);
            graphic.kind = kind;
            graphic.resource = resource;
            let mut op = Retrieval::new(3, Some(7));
            if let Some(request) = op
                .advance(&mut cache, &mut graphic, false, &mut |_, _| {})
                .unwrap()
            {
                assert_eq!(request, Request::Definition { resource: 1 });
                op.respond(
                    &mut cache,
                    &mut graphic,
                    Response::Definition(Definition {
                        group: 0,
                        base_graphic: 0,
                        icon_base: 123,
                    }),
                    &mut |_, _| {},
                )
                .unwrap();
                assert_eq!(
                    op.advance(&mut cache, &mut graphic, false, &mut |_, _| {})
                        .unwrap(),
                    None
                );
            }
            assert_eq!(op.clone().finish().unwrap(), (true, Some(2)));
            cache.refs[1] = 1;
            assert!(op.validate(&cache, &graphic).is_err());
        }
    }
}
