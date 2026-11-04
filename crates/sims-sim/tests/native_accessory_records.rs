use serde_json::{Value, json};
use sims_sim::pose::{
    accessory::{Execution, Request},
    appearance::{Binding, Record, Records},
};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_accessory_duplicate_removal_append_and_failed_binding() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-accessory-records.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 60);
    for case in cases {
        let input = &case["input"];
        let mut records = Records::default();
        records.entries = input["records"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(id, r)| Record {
                binding: (r["bound"] == true).then_some(Binding {
                    id: id as u64 + 1,
                    owner: 1,
                }),
                name: r["name"].as_str().unwrap().into(),
                texture: r["texture"].as_str().unwrap().into(),
            })
            .collect();
        records.accessories = (6..records.entries.len()).collect();
        let mut execution = Execution::new(
            input["name"].as_str().unwrap().into(),
            (input["remove"] == false).then(|| input["texture"].as_str().unwrap().into()),
        );
        let mut events = vec![];
        let mut freed = vec![];
        while let Some(request) = execution.next(&mut records).unwrap() {
            // The exact pending operation survives restoration, including the
            // newly appended empty record before a successful/failed bind.
            let saved = serde_json::to_vec(&(&execution, &records)).unwrap();
            (execution, records) = serde_json::from_slice(&saved).unwrap();
            execution.validate(&records).unwrap();
            let before = records.clone();
            assert_eq!(execution.next(&mut records).unwrap(), Some(request.clone()));
            assert_eq!(records, before);
            match request {
                Request::Clear(id) => {
                    events.push(json!({"clear":id}));
                    records.entries[id].binding = None;
                }
                Request::Destroy(id) => {
                    events.extend([
                        json!({"destroy":id}),
                        json!({"clear":id}),
                        json!({"free":id}),
                    ]);
                    records.entries[id].binding = None;
                    freed.push(id);
                }
                Request::Bind {
                    record,
                    name,
                    texture,
                } => {
                    events.push(json!({"bind":{"record":record,"name":name,"texture":texture}}));
                    if input["success"] == true && !name.is_empty() {
                        records.entries[record] = Record {
                            binding: Some(Binding { id: 100, owner: 1 }),
                            name,
                            texture,
                        };
                    }
                }
            }
            execution.acknowledge(&mut records).unwrap();
        }
        assert!(execution.is_complete());
        assert_eq!(json!(events), case["events"], "{input}");
        let output: Vec<_> = records
            .entries
            .iter()
            .enumerate()
            .map(|(id, r)| {
                if freed.contains(&id) {
                    assert_eq!(*r, Record::default());
                    Value::Null
                } else {
                    json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture})
                }
            })
            .collect();
        assert_eq!(
            json!({"records":output,"accessories":records.accessories}),
            case["original"],
            "{input}"
        );
    }
}

#[test]
fn saved_accessory_request_cannot_target_body_or_change_binding_input() {
    let mut records = Records::default();
    let mut execution = Execution::new("suit-fork".into(), Some("".into()));
    execution.next(&mut records).unwrap();
    execution.validate(&records).unwrap();
    let saved = serde_json::to_value(&execution).unwrap();
    for (field, value) in [
        ("record", json!(records.body[0])),
        ("texture", json!("different")),
        ("name", json!("other")),
    ] {
        let mut bad = saved.clone();
        bad["pending"]["Bind"][field] = value;
        let bad: Execution = serde_json::from_value(bad).unwrap();
        assert!(bad.validate(&records).is_err());
    }
    let mut duplicate = records.clone();
    duplicate.accessories.push(6);
    assert!(execution.validate(&duplicate).is_err());
    let mut body = records.clone();
    body.accessories.push(body.body[0]);
    assert!(execution.validate(&body).is_err());
}
