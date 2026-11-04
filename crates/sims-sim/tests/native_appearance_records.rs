use serde_json::{Value, json};
use sims_sim::pose::appearance::{Binding, Execution, Input, Record, Records, Request, Response};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_appearance_record_services_and_saved_continuations() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-appearance-records.json")).unwrap();
    let mut services = 0;
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut record = Record {
            binding: (c["old"] == true).then_some(Binding { id: 1, owner: 1 }),
            name: c["name"].as_str().unwrap().into(),
            texture: c["texture"].as_str().unwrap().into(),
        };
        let mut execution = if c["clear"] == true {
            Execution::clear()
        } else {
            Execution::bind(if c["current"] == true {
                Input::Current
            } else {
                Input::Replace {
                    name: c["replace"][0].as_str().unwrap().into(),
                    texture: c["replace"][1].as_str().unwrap().into(),
                }
            })
        };
        let mut events = vec![];
        let mut dirty = [0; 3];
        while let Some(request) = execution.next(&mut record) {
            let bytes = serde_json::to_vec(&(&execution, &record)).unwrap();
            let restored: (Execution, Record) = serde_json::from_slice(&bytes).unwrap();
            assert_eq!((&execution, &record), (&restored.0, &restored.1));
            (execution, record) = restored;
            assert_eq!(execution.next(&mut record), Some(request.clone()));
            let before = (execution.clone(), record.clone());
            let wrong = match request {
                Request::Lookup { .. } => Response::Binding(None),
                _ => Response::Asset(None),
            };
            assert!(execution.acknowledge(&mut record, wrong).is_err());
            assert_eq!((&execution, &record), (&before.0, &before.1));
            events.push(json!({"request":request,"record":record}));
            let stage = match request {
                Request::Lookup { .. } => "lookup",
                Request::Bind { .. } => "bind",
                Request::Unbind { .. } => "unbind",
                Request::Dirty { .. } => "dirty",
            };
            let mutation = &c["mutation"];
            if mutation["stage"] == stage {
                record.name = mutation["name"].as_str().unwrap().into();
                record.texture = mutation["texture"].as_str().unwrap().into();
                record.binding =
                    (mutation["binding"] == true).then_some(Binding { id: 3, owner: 3 });
            }
            let response = match request {
                Request::Lookup { .. } => Response::Asset((c["asset"] == true).then_some(1)),
                Request::Bind { .. } => {
                    Response::Binding((c["created"] == true).then_some(Binding { id: 2, owner: 2 }))
                }
                Request::Unbind { .. } => Response::Done,
                Request::Dirty { owner } => {
                    dirty[owner as usize - 1] = 1;
                    Response::Done
                }
            };
            execution.acknowledge(&mut record, response).unwrap();
            services += 1;
        }
        assert_eq!(json!(events), row["events"], "service order {n}");
        assert_eq!(
            json!({"record":record,"result":execution.result().unwrap(),"dirty":dirty}),
            row["original"],
            "state {n}"
        );
    }
    eprintln!("128 native appearance cases; {services} service boundaries");
    assert_eq!(report["cases"].as_array().unwrap().len(), 128);
    assert!(services > 150);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn appearance_record_allocation_order_matches_native_constructor() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-animator-construction.json")).unwrap();
    let records = Records::default();
    for row in report["cases"].as_array().unwrap() {
        let expected = &row["original"];
        assert_eq!(json!([records.generic]), expected["vectors"]["192"]);
        assert_eq!(json!(records.body), expected["vectors"]["604"]);
        assert_eq!(json!(records.hands), expected["vectors"]["616"]);
        assert_eq!(json!(records.accessories), expected["vectors"]["628"]);
        assert_eq!(json!(records.head), expected["single_record"]);
    }
    assert!(records.entries.iter().all(|r| r == &Record::default()));
}
