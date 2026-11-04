use serde_json::{Value, json};
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    appearance_reload::{Owner, Reload, Request},
    appearance_selection::{Request as SelectRequest, Selection, State},
};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_reload_preserves_clear_selection_identity_and_destruction_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-appearance-reload.json")).unwrap();
    for row in report["constructor_tints"].as_array().unwrap() {
        let owner = Owner::constructed(false);
        assert_eq!(json!(owner.selection.tint), row["tint"]);
        assert_eq!(json!([0, owner.attachment_metadata]), row["attachment"]);
        assert!(owner.attachment.is_none());
    }
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let profile: Vec<String> = serde_json::from_value(c["profile"].clone()).unwrap();
        let tones: Vec<String> = serde_json::from_value(c["tones"].clone()).unwrap();
        let mut person: std::collections::BTreeMap<i16, i16> = c["person"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(n, v)| (n.parse().unwrap(), v.as_i64().unwrap() as i16))
            .collect();
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
        if c["accessories"].as_bool().unwrap() {
            records.accessories = vec![6, 7];
        }
        let mut owner = Owner {
            selection: State::constructed([23, 47, 91]),
            model_present: c["model"].as_bool().unwrap(),
            attachment: c["attachment"].as_bool().unwrap().then_some(77),
            attachment_metadata: 91,
        };
        owner.selection.cached = serde_json::from_value(c["cache"].clone()).unwrap();
        let mut reload = Reload::begin(&mut owner);
        let mut events = vec![];
        while let Some(request) = reload.next(&mut owner, &records, &mut person).unwrap() {
            let before = (
                reload.clone(),
                owner.clone(),
                records.clone(),
                person.clone(),
            );
            (reload, owner, records, person) =
                serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
            assert_eq!(
                (
                    reload.clone(),
                    owner.clone(),
                    records.clone(),
                    person.clone()
                ),
                before
            );
            assert_eq!(
                reload.next(&mut owner, &records, &mut person).unwrap(),
                Some(request.clone())
            );
            assert_eq!(
                (
                    reload.clone(),
                    owner.clone(),
                    records.clone(),
                    person.clone()
                ),
                before
            );
            let words = person.values().copied().collect::<Vec<_>>();
            match request {
                Request::ClearRecord(id) => {
                    events.push(json!({"clear":id}));
                    records.entries[id].binding = None;
                }
                Request::Detach(id) => events.push(json!({"detach":id})),
                Request::BindProfile => events.push(json!({"bind_profile":words})),
                Request::SelectAppearance => {
                    events.push(json!({"select":words}));
                    let mut selection =
                        Selection::begin(&mut owner.selection, &mut person, &profile, 0, false)
                            .unwrap();
                    while let Some(request) = selection.next(&owner.selection, &records) {
                        selection =
                            serde_json::from_slice(&serde_json::to_vec(&selection).unwrap())
                                .unwrap();
                        assert_eq!(
                            selection.next(&owner.selection, &records),
                            Some(request.clone())
                        );
                        match request {
                            SelectRequest::Bind { record, descriptor } => {
                                events.push(json!({"bind":{"record":record,"name":descriptor.name,"texture":descriptor.texture}}));
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
                            SelectRequest::TextureNotification { first } => {
                                events.push(json!({"texture_notification":if first {"4eb0cc"} else {"4eb0eb"},"guid":0x12345678u32}));
                                if c["mutate_hands"].as_bool().unwrap_or(false) {
                                    person.insert(22, -8);
                                    person.insert(23, 4);
                                }
                            }
                            SelectRequest::Tint(tint) => events.push(json!({"tint":tint})),
                        }
                        selection.acknowledge().unwrap();
                    }
                }
                Request::IdentityFields => {
                    events.push(json!({"fields":words}));
                    sims_vm::person_reset::reload_profile_identity_fields(
                        &mut person,
                        &profile.iter().map(|s| Some(s.as_str())).collect::<Vec<_>>(),
                        &tones.iter().map(String::as_str).collect::<Vec<_>>(),
                    )
                    .unwrap();
                }
                Request::DestroyAccessory(id) => {
                    events.push(json!({"destroy":id,"person":words}));
                    events.push(json!({"clear":id}));
                    events.push(json!({"free":id}));
                    records.entries[id].binding = None;
                }
            }
            reload.acknowledge(&mut owner, &mut records).unwrap();
        }
        assert!(reload.is_complete());
        assert_eq!(json!(events), row["events"], "{c}");
        let actual = json!({"person":person.values().copied().collect::<Vec<_>>(),"cache":owner.selection.cached,
            "dirty":owner.selection.dirty,"special_dirty":owner.selection.special_dirty,"skin_dirty":owner.selection.skin_dirty,
            "attachment":owner.attachment.is_some(),"attachment_metadata":owner.attachment_metadata,
            "accessories":records.accessories.len(),"records":records.entries[..6].iter().map(|r|
            json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture})).collect::<Vec<_>>()});
        assert_eq!(actual, row["original"], "{c}");
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 49);
}
