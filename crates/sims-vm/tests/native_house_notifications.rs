use serde_json::{Value, json};
use sims_vm::commander::{
    Registry,
    house::{
        Fields, Receiver, Request,
        dispatch::{Dispatch, Progress, Trace},
    },
};
use std::collections::BTreeMap;

#[test]
fn history_notice_only_marks_controls_and_ignores_creation_gate() {
    use sims_vm::commander::house::Call;
    for gate in [0, 7] {
        for creation_gate in [0, 9] {
            let mut fields = Fields {
                viewer_interface: true,
                include_visible: 1,
                renderer_dirty: 0,
                ui: sims_vm::commander::house::Interface {
                    mask_owner_present: true,
                    panel_active: true,
                    shared_present: true,
                    gated: gate,
                    creation_gated: creation_gate,
                    dirty: 0,
                },
            };
            let mut globals = BTreeMap::new();
            for receiver in [
                Receiver::Base,
                Receiver::World,
                Receiver::Module,
                Receiver::Objects,
                Receiver::Globals,
                Receiver::Lot,
                Receiver::Viewer,
                Receiver::Renderer,
            ] {
                let before = fields.clone();
                let c =
                    Call::begin(receiver, 0x123400ee, 99, true, &mut fields, &mut globals).unwrap();
                c.validate().unwrap();
                assert_eq!(c.returned(), Some(0));
                assert_eq!(fields, before);
            }
            let mut c = Call::begin(
                Receiver::Interface,
                0xee,
                99,
                true,
                &mut fields,
                &mut globals,
            )
            .unwrap();
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            c.validate().unwrap();
            if gate == 0 {
                assert_eq!(fields.ui.dirty, 1);
                assert_eq!(
                    c.advance(&fields, &globals).unwrap(),
                    Some(&Request::UiFlags { bits: 4 })
                );
                fields.ui.gated = 12;
                c.acknowledge(&Request::UiFlags { bits: 4 }).unwrap();
                assert_eq!(c.advance(&fields, &globals).unwrap(), None);
            } else {
                assert_eq!(fields.ui.dirty, 0);
            }
            c.validate().unwrap();
            assert_eq!(c.returned(), Some(1));
        }
    }
}
fn snapshot(fields: &Fields, globals: &BTreeMap<i16, i16>) -> Value {
    let mut fields = serde_json::to_value(fields).unwrap();
    fields["ui"]["word44"] = json!(globals[&26]);
    json!({"fields":fields,"globals":[globals[&2],globals[&3]]})
}
fn record(
    fields: &Fields,
    globals: &BTreeMap<i16, i16>,
    events: &mut Vec<Value>,
    depth: usize,
    key: &str,
    value: Value,
) {
    let mut event = snapshot(fields, globals);
    event["depth"] = json!(depth);
    event[key] = value;
    events.push(event);
}
fn patch(target: &mut Value, input: &Value) {
    if let Some(m) = input.as_object() {
        for (k, v) in m {
            patch(&mut target[k], v)
        }
    } else {
        *target = input.clone();
    }
}
fn run(
    d: &mut Dispatch,
    g: &mut BTreeMap<i16, i16>,
    c: &Value,
    services: &mut usize,
    events: &mut Vec<Value>,
    nested: bool,
    budget: usize,
) -> u16 {
    for _ in 0..5000 {
        // Restore the complete production stack, including a viewer callback
        // suspended inside its own nested notification.
        (*d, *g) = serde_json::from_slice(&serde_json::to_vec(&(&d, &g)).unwrap()).unwrap();
        d.validate().unwrap();
        let selected = c["selected"].as_bool().unwrap();
        let p = d
            .advance(g, selected, budget, |depth, t, f, g| match t {
                Trace::Enter { event, value } => record(
                    f,
                    g,
                    events,
                    depth,
                    "enter",
                    json!({"event":event,"value":value}),
                ),
                Trace::Invoke(i) => record(
                    f,
                    g,
                    events,
                    depth,
                    "invoke",
                    serde_json::to_value(i).unwrap(),
                ),
                Trace::Returned(n) => record(f, g, events, depth, "returned", json!(n)),
            })
            .unwrap();
        match p {
            Progress::Pending => (),
            Progress::Complete(n) | Progress::NestedComplete(n) => {
                assert_eq!(matches!(p, Progress::NestedComplete(_)), nested);
                let before = d.clone();
                assert_eq!(
                    d.advance(g, selected, budget, |_, _, _, _| panic!(
                        "repeated return trace"
                    ))
                    .unwrap(),
                    p
                );
                assert_eq!(*d, before);
                return n;
            }
            Progress::Service(ref service) => {
                let before = d.clone();
                assert_eq!(
                    d.advance(g, selected, budget, |_, _, _, _| panic!(
                        "repeated callback trace"
                    ))
                    .unwrap(),
                    p
                );
                let mut bad = service.clone();
                bad.invocation.recipient += 100;
                assert!(d.acknowledge(&bad).is_err());
                assert_eq!(*d, before);
                let mut orphan = serde_json::to_value(&*d).unwrap();
                orphan["stack"][d.depth() - 1]["broadcast"]["pending"] = Value::Null;
                assert!(
                    serde_json::from_value::<Dispatch>(orphan)
                        .unwrap()
                        .validate()
                        .is_err()
                );
                record(
                    &d.fields,
                    g,
                    events,
                    d.depth(),
                    "service",
                    serde_json::to_value(&service.request).unwrap(),
                );
                for change in c["changes"].as_array().unwrap() {
                    if change["service"].as_u64().unwrap() as usize == *services {
                        let mut f = snapshot(&d.fields, g)["fields"].clone();
                        patch(&mut f, &change["fields"]);
                        g.insert(26, f["ui"]["word44"].as_i64().unwrap() as i16);
                        d.fields = serde_json::from_value(f).unwrap();
                    }
                }
                *services += 1;
                if service.request == Request::ViewerRefresh && c["viewer_broadcasts"] == true {
                    for (event, value) in [(0xf9, 2), (0x84, 0)] {
                        d.nest(event, value).unwrap();
                        assert!(d.acknowledge(service).is_err());
                        assert!(d.finish_nested().is_err());
                        let result = run(d, g, c, services, events, true, budget);
                        assert_eq!(d.finish_nested().unwrap(), result);
                        assert_eq!(d.pending().as_ref(), Some(service));
                    }
                }
                d.acknowledge(service).unwrap();
            }
        }
    }
    panic!("house notification did not return");
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_house_listeners_live_fields_and_nested_selection_broadcast() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/house-notifications.json")).unwrap();
    compare_house_corpus(report);
}

#[test]
#[ignore = "private guarded original83 notification corpus"]
fn original_drawing_notice_invalidates_before_terrain_and_preserves_other_listeners() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/mode-notifications.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    for row in report["cases"].as_array().unwrap() {
        assert_eq!(row["input"]["event"].as_u64().unwrap() as u16, 0x83);
    }
    compare_house_corpus(report);
}

#[test]
#[ignore = "requires guarded private native_transport_notifications.py corpus"]
fn original_transport_notice_traverses_house_listeners_without_fake_ui_work() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/transport-notifications.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    for row in report["cases"].as_array().unwrap() {
        assert_eq!(row["input"]["event"].as_u64().unwrap() as u16, 0xc2);
        assert_eq!(row["original"]["returned"], 0);
    }
    compare_house_corpus(report);
}

fn compare_house_corpus(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 192);
    let receivers = [
        Receiver::World,
        Receiver::Module,
        Receiver::Objects,
        Receiver::Globals,
        Receiver::Lot,
        Receiver::Viewer,
        Receiver::Renderer,
        Receiver::Interface,
    ];
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut r = Registry::default();
        r.generation = c["generation"].as_u64().unwrap() as u32;
        for id in c["order"].as_array().unwrap() {
            let id = id.as_u64().unwrap() as u32;
            r.construct(id).unwrap();
            r.bind(id, receivers[id as usize - 1]).unwrap();
        }
        let fields: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
        let globals = BTreeMap::from([
            (2, c["globals"][0].as_i64().unwrap() as i16),
            (3, c["globals"][1].as_i64().unwrap() as i16),
            (26, c["fields"]["ui"]["word44"].as_i64().unwrap() as i16),
        ]);
        let mut results = vec![];
        for budget in [1, 10000] {
            let mut d = Dispatch::begin(
                r.clone(),
                fields.clone(),
                c["event"].as_u64().unwrap() as u32,
                c["value"].as_u64().unwrap() as u32,
            )
            .unwrap();
            let mut g = globals.clone();
            let mut events = vec![];
            let returned = run(&mut d, &mut g, c, &mut 0, &mut events, false, budget);
            let mut output = snapshot(&d.fields, &g);
            output["events"] = json!(events);
            output["returned"] = json!(returned);
            assert_eq!(
                output["events"].as_array().unwrap().len(),
                row["original"]["events"].as_array().unwrap().len(),
                "case {n} count"
            );
            for (i, event) in output["events"].as_array().unwrap().iter().enumerate() {
                assert_eq!(
                    event, &row["original"]["events"][i],
                    "case {n}, event {i}, budget {budget}"
                );
            }
            assert_eq!(
                output, row["original"],
                "house broadcast {n}, budget {budget}"
            );
            results.push((d, g));
        }
        assert_eq!(results[0], results[1]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn distinct_globals_instances_cannot_alias_one_house_context() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/house-notifications.json")).unwrap();
    let fields: Fields =
        serde_json::from_value(report["cases"][0]["input"]["fields"].clone()).unwrap();
    let mut registry = Registry::default();
    for id in [1, 2] {
        registry.construct(id).unwrap();
        registry.bind(id, Receiver::Globals).unwrap();
    }
    assert!(Dispatch::begin(registry, fields, 0xe4, 7).is_err());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_created_broadcast_uses_full_id_and_distinct_interface_gate() {
    use sims_vm::{Definition, Module, Object, Program, World, person::WorldState};
    let data: Value =
        serde_json::from_str(include_str!("fixtures/created-notifications.json")).unwrap();
    assert_eq!(data["cases"].as_array().unwrap().len(), 192);
    assert_eq!(
        data["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let handlers = [
        Receiver::World,
        Receiver::Module,
        Receiver::Objects,
        Receiver::Globals,
        Receiver::Lot,
        Receiver::Viewer,
        Receiver::Renderer,
        Receiver::Interface,
    ];
    for (n, row) in data["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        for budget in [1, 10000] {
            let mut r = Registry::default();
            r.generation = c["generation"].as_u64().unwrap() as u32;
            for id in c["order"].as_array().unwrap() {
                let id = id.as_u64().unwrap() as u32;
                r.construct(id).unwrap();
                r.bind(id, handlers[id as usize - 1]).unwrap();
            }
            let mut d = Dispatch::begin(
                r,
                serde_json::from_value(c["fields"].clone()).unwrap(),
                c["event"].as_u64().unwrap() as u32,
                c["value"].as_u64().unwrap() as u32,
            )
            .unwrap();
            let mut g = BTreeMap::from([
                (2, -11),
                (3, 23),
                (26, c["fields"]["ui"]["word44"].as_i64().unwrap() as i16),
            ]);
            let mut world = World::default();
            let mut program = Program::default();
            let mut module = Module {
                source: "created".into(),
                ..Default::default()
            };
            for (i, kind) in c["kinds"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .chain(std::iter::once((98, &json!(2))))
            {
                if kind.is_null() {
                    continue;
                }
                let id = (i + 1) as i16;
                let mut w = vec![0; 128];
                w[9] = kind.as_i64().unwrap() as i16;
                module.object_definition_words.insert(id as u16, w);
                world.objects.insert(
                    id,
                    Object {
                        definition: Some(Definition {
                            source: "created".into(),
                            object: id as u16,
                            guid: id as u32,
                            slots: 0,
                        }),
                        person: (id == 99).then(|| BTreeMap::from([(32, 0)])),
                        ..Default::default()
                    },
                );
            }
            program.modules.insert("created".into(), module);
            world.globals.insert(10, 1);
            world.person = Some(WorldState {
                controlled_family: 1,
                families: BTreeMap::from([(1, vec![])]),
                person_order: vec![99],
                lot_zoning: BTreeMap::from([(1, 1)]),
                ..Default::default()
            });
            let mut events = vec![];
            let mut services = 0;
            for _ in 0..1000 {
                d = serde_json::from_slice(&serde_json::to_vec(&d).unwrap()).unwrap();
                d.validate().unwrap();
                match d
                    .advance(&mut g, false, budget, |depth, t, f, g| match t {
                        Trace::Enter { event, value } => record(
                            f,
                            g,
                            &mut events,
                            depth,
                            "enter",
                            json!({"event":event,"value":value}),
                        ),
                        Trace::Invoke(i) => record(
                            f,
                            g,
                            &mut events,
                            depth,
                            "invoke",
                            serde_json::to_value(i).unwrap(),
                        ),
                        Trace::Returned(n) => {
                            record(f, g, &mut events, depth, "returned", json!(n))
                        }
                    })
                    .unwrap()
                {
                    Progress::Pending => (),
                    Progress::Complete(_) => break,
                    Progress::NestedComplete(_) => panic!("unexpected nested creation"),
                    Progress::Service(call) => {
                        record(
                            &d.fields,
                            &g,
                            &mut events,
                            d.depth(),
                            "service",
                            serde_json::to_value(&call.request).unwrap(),
                        );
                        for change in c["changes"].as_array().unwrap() {
                            if change["service"].as_u64().unwrap() as usize == services {
                                let mut f = snapshot(&d.fields, &g)["fields"].clone();
                                patch(&mut f, &change["fields"]);
                                g.insert(26, f["ui"]["word44"].as_i64().unwrap() as i16);
                                d.fields = serde_json::from_value(f).unwrap();
                            }
                        }
                        services += 1;
                        let Request::WorldCreated { value } = call.request else {
                            panic!("unexpected E2 service")
                        };
                        sims_vm::family::created(&mut world, &program, value).unwrap();
                        d.acknowledge(&call).unwrap();
                    }
                }
            }
            let mut result = snapshot(&d.fields, &g);
            result["events"] = json!(events);
            result["returned"] = json!(d.returned().unwrap());
            //42a2b4 is an explicit native probe boundary; its production Rust
            // implementation is independently compared by native_family_removal.
            result["visitor_calls"] = json!(world.objects[&99].person.as_ref().unwrap()[&32]);
            assert_eq!(result, row["original"], "created case {n}, budget {budget}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn speed_override_notice_keeps_unbound_windows_panel_explicit() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/house-notifications.json")).unwrap();
    let mut fields: Fields =
        serde_json::from_value(report["cases"][0]["input"]["fields"].clone()).unwrap();
    let mut globals = BTreeMap::from([(2, 1), (3, 2), (26, 0)]);
    let before = (fields.clone(), globals.clone());
    for receiver in [
        Receiver::Base,
        Receiver::World,
        Receiver::Module,
        Receiver::Objects,
        Receiver::Globals,
        Receiver::Lot,
        Receiver::Viewer,
        Receiver::Renderer,
    ] {
        let call = sims_vm::commander::house::Call::begin(
            receiver,
            0xffff0109,
            0xffffffff,
            false,
            &mut fields,
            &mut globals,
        )
        .unwrap();
        assert_eq!(call.returned(), Some(0));
        assert_eq!((&fields, &globals), (&before.0, &before.1));
    }
    assert!(
        sims_vm::commander::house::Call::begin(
            Receiver::Interface,
            0x109,
            1,
            false,
            &mut fields,
            &mut globals
        )
        .is_err()
    );
    assert_eq!((fields, globals), before);
}

#[test]
#[ignore = "private guarded original109 broadcast comparison, seven house listeners"]
fn original_speed_override_notice_matches_hosted_house_listeners() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/clock-notifications.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    for row in report["cases"].as_array().unwrap() {
        assert_eq!(row["input"]["event"].as_u64().unwrap() as u16, 0x109);
        assert!(
            !row["input"]["order"]
                .as_array()
                .unwrap()
                .contains(&json!(8))
        );
    }
    compare_house_corpus(report);
}
