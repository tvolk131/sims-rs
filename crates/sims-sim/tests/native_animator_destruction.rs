use serde_json::{Value, json};
use sims_sim::pose::destruction::{Execution, Request, State, Storage};

fn name(id: u64) -> String {
    match id {
        10 => "model".into(),
        11 => "profile".into(),
        40 => "head".into(),
        80 => "generic".into(),
        20..=23 => format!("layer{}", id - 20),
        30..=33 => format!("anchor{}", id - 30),
        50..=52 => format!("accessory{}", id - 50),
        60..=61 => format!("hand{}", id - 60),
        70..=71 => format!("body{}", id - 70),
        _ => panic!("unknown fixture identity {id}"),
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_animator_destruction_orders_services_and_retains_pending_owners() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-animator-destruction.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 128);
    for row in fixture["cases"].as_array().unwrap() {
        let c = &row["input"];
        let flag = |k: &str| c[k].as_bool().unwrap();
        let mask = |k: &str, i: usize| c[k].as_u64().unwrap() & (1 << i) != 0;
        let mut s = State {
            model: flag("model").then_some(10),
            profile: flag("profile").then_some(11),
            layers: std::array::from_fn(|i| {
                (flag("model") && mask("layers", i)).then_some(20 + i as u64)
            }),
            anchors: std::array::from_fn(|i| mask("anchors", i).then_some(30 + i as u64)),
            head: flag("head").then_some(40),
            accessories: (0..c["accessories"].as_u64().unwrap())
                .map(|i| (i != 1).then_some(50 + i))
                .collect(),
            hands: (0..2)
                .map(|i| mask("hands", i).then_some(60 + i as u64))
                .collect(),
            bodies: (0..2)
                .map(|i| mask("bodies", i).then_some(70 + i as u64))
                .collect(),
            generic: if flag("generic") {
                vec![Some(80), None]
            } else {
                vec![]
            },
            selected: c["selected"] == 2,
            focus: flag("focus"),
            base_focus: flag("base_focus"),
        };
        let mut e = Execution::default();
        let mut events = vec![];
        while let Some(request) = e.next(&mut s).unwrap() {
            (e, s) = serde_json::from_slice(&serde_json::to_vec(&(&e, &s)).unwrap()).unwrap();
            let before = (e.clone(), s.clone());
            assert_eq!(e.next(&mut s).unwrap(), Some(request.clone()));
            assert_eq!((e.clone(), s.clone()), before);
            if request != Request::BaseReset {
                assert!(e.acknowledge(&mut s, &Request::BaseReset).is_err());
                assert_eq!((e.clone(), s.clone()), before);
            }
            match &request {
                Request::Reset => {
                    events.push(json!(["reset", "animator"]));
                    if flag("reset_clears") {
                        s.layers[0] = None;
                        s.layers[1] = None;
                    }
                }
                Request::ClearAppearances => events.push(json!(["clear_appearances", "animator"])),
                Request::ReleaseProfile(id) => events.push(json!(["release_profile", name(*id)])),
                Request::DetachLayer(id) => events.push(json!(["detach_layer", name(*id)])),
                Request::DestroyAnchor(id)
                | Request::DestroyModel(id)
                | Request::DestroyRecord(id) => {
                    let op = match request {
                        Request::DestroyAnchor(_) => "destroy_anchor",
                        Request::DestroyModel(_) => "destroy_model",
                        _ => "destroy_record",
                    };
                    events.push(json!([op, name(*id)]));
                    events.push(json!(["free", name(*id)]));
                }
                Request::ClearSelected => events.push(json!(["clear_selected"])),
                Request::ClearFocus => events.push(json!(["clear_focus"])),
                Request::BaseReset => events.push(json!(["base_reset", "animator"])),
                Request::ClearBaseFocus => events.push(json!(["clear_base_focus"])),
                Request::BaseClear => events.push(json!(["base_clear", "animator"])),
                Request::Storage(Storage::Name) => events.push(json!(["destroy_string", "name"])),
                Request::Storage(storage) => events.push(json!([
                    "destroy_vector",
                    match storage {
                        Storage::Accessories => 0x274,
                        Storage::Hands => 0x268,
                        Storage::Bodies => 0x25c,
                        Storage::BaseAuxiliaryRecords => 0xcc,
                        Storage::Generic => 0xc0,
                        Storage::BaseLayers => 0x2c,
                        Storage::Name => unreachable!(),
                    }
                ])),
            }
            e.acknowledge(&mut s, &request).unwrap();
        }
        assert!(e.is_complete());
        assert_eq!(json!(events), row["original"]["events"], "{c}");
        let pointers = |ids: &[Option<u64>]| {
            ids.iter()
                .map(|id| {
                    if id.is_none() {
                        0
                    } else {
                        panic!("retained live identity")
                    }
                })
                .collect::<Vec<_>>()
        };
        let remaining = json!({"0x8":s.model.unwrap_or(0),"0xc":s.layers[0].unwrap_or(0),"0x10":s.layers[1].unwrap_or(0),
            "0x13c":s.layers[2].unwrap_or(0),"0x14":s.layers[3].unwrap_or(0),
            "0x14c":s.anchors[0].unwrap_or(0),"0x144":s.anchors[1].unwrap_or(0),
            "0x140":s.anchors[2].unwrap_or(0),"0x148":s.anchors[3].unwrap_or(0),"0x258":s.head.unwrap_or(0),
            "0x268":pointers(&s.hands),"0x25c":pointers(&s.bodies),"0x274":pointers(&s.accessories),"0xc0":pointers(&s.generic)});
        assert_eq!(remaining, row["original"]["remaining"], "{c}");
    }
}
