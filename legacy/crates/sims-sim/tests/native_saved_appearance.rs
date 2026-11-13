use serde_json::{Value, json};
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    appearance_read::{Read, Request},
    appearance_reload::{Owner, Request as Lifecycle},
    appearance_selection::Descriptor,
};

fn compare(report: &Value) -> usize {
    let mut services = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut records = Records::default();
        records.entries = c["records"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(id, r)| Record {
                binding: r["bound"].as_bool().unwrap().then_some(Binding {
                    id: id as u64 + 1,
                    owner: 1,
                }),
                name: r["name"].as_str().unwrap().into(),
                texture: r["texture"].as_str().unwrap().into(),
            })
            .collect();
        let old_accessories: Vec<_> = (6..records.entries.len()).collect();
        records.accessories = old_accessories.clone();
        let mut owner = Owner::constructed(true);
        owner.selection.cached = serde_json::from_value(c["cache"].clone()).unwrap();
        owner.attachment = c["attachment"].as_bool().unwrap().then_some(77);
        owner.attachment_metadata = c["attachment_metadata"].as_u64().unwrap_or(91) as u32;
        let pair = |v: &Value| Descriptor {
            name: v[0].as_str().unwrap().into(),
            texture: v[1].as_str().unwrap().into(),
        };
        let fixed = std::array::from_fn(|i| pair(&c["fixed"][i]));
        let accessories = c["saved_accessories"]
            .as_array()
            .unwrap()
            .iter()
            .map(pair)
            .collect();
        let mut read = Read::new(
            &records,
            fixed,
            accessories,
            c["skip_bind"].as_bool().unwrap(),
        )
        .unwrap();
        let mut events = vec![];
        while let Some(request) = read.next(&mut owner, &mut records).unwrap() {
            let before = (read.clone(), owner.clone(), records.clone());
            (read, owner, records) =
                serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
            assert_eq!((&read, &owner, &records), (&before.0, &before.1, &before.2));
            assert_eq!(
                read.next(&mut owner, &mut records).unwrap(),
                Some(request.clone())
            );
            assert_eq!((&read, &owner, &records), (&before.0, &before.1, &before.2));
            match request {
                Request::Lifecycle(Lifecycle::ClearRecord(id)) => {
                    events.push(json!({"clear":id}));
                    records.entries[id].binding = None;
                }
                Request::Lifecycle(Lifecycle::DestroyAccessory(id)) => {
                    events.extend([
                        json!({"destroy":id}),
                        json!({"clear":id}),
                        json!({"free":id}),
                    ]);
                    records.entries[id].binding = None;
                }
                Request::Lifecycle(Lifecycle::Detach(id)) => events.push(json!({"detach":id})),
                Request::BindRecord(id) => {
                    let record = &mut records.entries[id];
                    events.push(
                        json!({"bind":{"record":id,"name":record.name,"texture":record.texture}}),
                    );
                    if !record.name.is_empty() {
                        record.binding = Some(Binding {
                            id: id as u64 + 1,
                            owner: 1,
                        });
                    }
                }
                Request::TextureNotification => {
                    events.push(json!({"texture_notification":"4eb0eb","guid":0x12345678u32}))
                }
                other => panic!("unexpected read service {other:?}"),
            }
            read.acknowledge(&mut owner, &mut records).unwrap();
            services += 1;
        }
        assert!(read.is_complete());
        assert_eq!(json!(events), row["events"]);
        let stored: Vec<_> = records
            .entries
            .iter()
            .enumerate()
            .map(|(id, r)| {
                if old_accessories.contains(&id) {
                    assert_eq!(r, &Record::default());
                    Value::Null
                } else {
                    json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture})
                }
            })
            .collect();
        assert_eq!(
            json!({"records":stored,"accessories":records.accessories,"dirty":owner.selection.dirty,
            "attachment":owner.attachment.is_some(),"attachment_metadata":owner.attachment_metadata,
            "cache":owner.selection.cached}),
            row["original"]
        );
    }
    services
}

#[cfg(feature = "private-fixtures")]
#[test]
fn saved_appearance_reader_matches_native_order_and_head_notification() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-saved-appearance.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 144);
    assert!(compare(&report) > 1500);
}

#[test]
#[ignore = "requires private original saved-appearance native evidence"]
fn original_saved_appearance_prefix_matches_native() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-original-saved-appearance-prefix.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 7);
    assert!(compare(&report) >= 70);
}
