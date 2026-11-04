//! Prefix control-flow evidence and selector output are separate from a full
//! native draw: virtual profile reload and graphics getters are supplied calls.
use serde_json::{Value, json};
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    appearance_selection::{Request, Selection, State},
};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_draw_prefix_always_selects_without_force_and_swimwear_uses_profile_fallback() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-person-draw-appearance.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 12);
    for row in report["cases"].as_array().unwrap() {
        let input = &row["input"];
        let prefix: Vec<_> = row["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e.get("draw_selection_force").is_some())
            .collect();
        assert_eq!(prefix, vec![&json!({"draw_selection_force":0})]);
        let reload: Vec<_> = row["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e.get("profile_reload_boundary").is_some())
            .collect();
        assert_eq!(
            reload.len(),
            usize::from(input["profile_dirty"].as_bool().unwrap())
        );
        if let Some(reload) = reload.first() {
            assert_eq!(
                reload["profile_reload_boundary"]["dirty_already_cleared"],
                true
            );
        }
        let profile: Vec<String> = serde_json::from_value(input["profile"].clone()).unwrap();
        assert_eq!(profile[31], "");
        assert_eq!(profile[16], "nmfat_01,BODY=umfatlgt_lguard01");
        let mut person = serde_json::from_value(input["person"].clone()).unwrap();
        let mut state = State::constructed([23, 47, 91]);
        state.cached = serde_json::from_value(input["cache"].clone()).unwrap();
        let mut records = Records::default();
        for (i, r) in input["records"].as_array().unwrap().iter().enumerate() {
            records.entries[i] = Record {
                binding: r["bound"].as_bool().unwrap().then_some(Binding {
                    id: i as u64 + 1,
                    owner: 1,
                }),
                name: r["name"].as_str().unwrap().into(),
                texture: r["texture"].as_str().unwrap().into(),
            };
        }
        let mut selection = Selection::begin(&mut state, &mut person, &profile, 0, false).unwrap();
        let mut events: Vec<_> = selection
            .lookups
            .iter()
            .map(|entry| json!({"profile":entry}))
            .collect();
        while let Some(request) = selection.next(&state, &records) {
            selection = serde_json::from_slice(&serde_json::to_vec(&selection).unwrap()).unwrap();
            match request {
                Request::Bind {record,descriptor} => {
                    events.push(json!({"bind":{"record":record,"name":descriptor.name,"texture":descriptor.texture}}));
                    if !descriptor.name.is_empty() { records.entries[record] = Record {
                        binding:Some(Binding {id:record as u64+1,owner:1}), name:descriptor.name,texture:descriptor.texture }; }
                }
                Request::TextureNotification {first} => events.push(json!({"texture_notification":if first {"4eb0cc"}else{"4eb0eb"},"guid":0x12345678u32})),
                Request::Tint(tint) => events.push(json!({"tint":tint})),
            }
            selection.acknowledge().unwrap();
        }
        let expected: Vec<_> = row["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                e.get("draw_selection_force").is_none()
                    && e.get("profile_reload_boundary").is_none()
            })
            .cloned()
            .collect();
        assert_eq!(events, expected);
        assert_eq!(json!(state.cached), row["original"]["cache"]);
        assert_eq!(json!(state.dirty), row["original"]["dirty"]);
        let person: std::collections::BTreeMap<i16, i16> = person;
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
