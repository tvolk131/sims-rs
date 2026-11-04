use serde_json::{Value, json};
use sims_sim::edit_history::{
    List, Store, Vector,
    object::{Class, Object, Objects, kind},
    snapshot::Snapshot,
};
use sims_vm::{
    World,
    budget::{Application, State},
    commander::{
        Registry,
        house::{
            Fields, Interface, Receiver, Request,
            dispatch::{Dispatch, Progress, Trace},
        },
    },
};
use std::collections::BTreeMap;

fn cold<T: serde::Serialize + serde::de::DeserializeOwned>(x: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(x).unwrap()).unwrap()
}
fn object(kind: i32) -> Object {
    Object {
        cost: None,
        budget: None,
        kind,
        special: None,
        snapshots: [Snapshot::default(), Snapshot::default()],
        class: Class::Object,
    }
}
fn funds(w: &World) -> Value {
    let b = w.budget.as_ref().unwrap();
    json!({"balance": b.balance, "expenses": b.expenses, "money": [w.globals[&13], w.globals[&14]]})
}
fn ui(f: &Fields) -> Value {
    json!([f.ui.gated, f.ui.creation_gated, f.ui.dirty])
}
fn run(c: &Value, restore: bool) -> Value {
    if c["op"] == "Kind" {
        let (mut s, mut o) = (Store::default(), Objects::default());
        for id in 1..=4 {
            s.construct_command(id).unwrap();
            s.lists.insert(List::Objects(id), Vector::default());
            o.0.insert(id, object(99 + id as i32));
        }
        for child in c["children"].as_array().unwrap() {
            s.append(List::Objects(1), child.as_u64().unwrap() as u32)
                .unwrap();
        }
        s.append(List::Objects(2), 4).unwrap();
        let mut call = kind::Call::new(1, c["value"].as_u64().unwrap() as u32);
        for _ in 0..100 {
            if restore {
                (s, o, call) = cold(&(s, o, call));
            }
            if call.advance(&s, &mut o, 1).unwrap() {
                return json!({"kinds": (1..=4).map(|id| o.0[&id].kind).collect::<Vec<_>>()});
            }
        }
        panic!("kind setter did not return");
    }
    let n = |key: &str| c[key].as_i64().unwrap() as i32;
    let mut w = World {
        budget: Some(State {
            balance: n("balance"),
            expenses: serde_json::from_value(c["expenses"].clone()).unwrap(),
            notifications: vec![777],
        }),
        globals: BTreeMap::from([(13, 123), (14, 456), (32, n("mode") as i16)]),
        ..Default::default()
    };
    let mut call = Application::new(n("category"), n("amount"));
    let mut f = Fields {
        viewer_interface: false,
        include_visible: 0,
        renderer_dirty: 0,
        ui: Interface {
            mask_owner_present: false,
            panel_active: false,
            shared_present: false,
            gated: n("gated") as u8,
            creation_gated: n("creation_gated") as u8,
            dirty: 7,
        },
    };
    let mut trace = vec![];
    if let Some(value) = call.advance(&mut w).unwrap() {
        if restore {
            (call, w) = cold(&(call, w));
        }
        let unchanged = serde_json::to_value(&w).unwrap();
        assert_eq!(call.advance(&mut w).unwrap(), Some(value));
        assert_eq!(serde_json::to_value(&w).unwrap(), unchanged);
        assert!(call.acknowledge(value.wrapping_add(1)).is_err());
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
        let mut r = Registry::default();
        for id in c["order"].as_array().unwrap() {
            let id = id.as_u64().unwrap() as u32;
            r.construct(id).unwrap();
            r.bind(id, receivers[id as usize - 1]).unwrap();
        }
        let mut d = Dispatch::begin(r, f, 0xda, value as u32).unwrap();
        let mut done = false;
        for _ in 0..100 {
            if restore {
                (d, call, w) = cold(&(d, call, w));
            }
            let current = funds(&w);
            match d
                .advance(&mut w.globals, false, 1, |_, event, _, _| match event {
                    Trace::Enter { event, value } => {
                        trace.push(json!(["Broadcast", event, value, current]))
                    }
                    Trace::Invoke(i) => trace.push(json!(["Listener", i.handler])),
                    Trace::Returned(_) => (),
                })
                .unwrap()
            {
                Progress::Pending => (),
                Progress::Complete(_) => {
                    done = true;
                    break;
                }
                Progress::Service(service) => {
                    match service.request {
                        Request::UiFlags { bits } => {
                            trace.push(json!(["Flags", bits, funds(&w), ui(&d.fields)]));
                            if c["mutate"] == true {
                                w.budget.as_mut().unwrap().balance = 123456;
                                d.fields.ui.gated = 255;
                                d.fields.ui.creation_gated = 255;
                            }
                        }
                        Request::UiRefresh => {
                            trace.push(json!(["Refresh", funds(&w), ui(&d.fields)]))
                        }
                        _ => panic!("unexpected budget service: {service:?}"),
                    }
                    d.acknowledge(&service).unwrap();
                }
                _ => panic!("unexpected nested budget broadcast"),
            }
        }
        assert!(done);
        f = d.fields;
        call.acknowledge(value).unwrap();
    }
    assert!(call.complete());
    assert_eq!(call.advance(&mut w).unwrap(), None);
    assert_eq!(w.budget.as_ref().unwrap().notifications, vec![777]);
    json!({"trace":trace,"funds":funds(&w),"ui":ui(&f)})
}
#[test]
fn direct_budget_uses_signed_wraparound_and_waits_for_original_listener_gates() {
    for gated in [0, 1, 255] {
        for creation_gated in [0, 1, 255] {
            let c = json!({"op":"Apply","mode":0,"balance":i32::MIN,"amount":1,"category":6,"expenses":[0,0,0,0,0,0,i32::MAX,0,0],"gated":gated,"creation_gated":creation_gated,"mutate":true,"order":[1,2,3,4,5,6,7,8]});
            let r = run(&c, true);
            assert_eq!(r, run(&c, false));
            let callbacks = r["trace"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[0] == "Flags" || e[0] == "Refresh")
                .count();
            assert_eq!(
                callbacks,
                if gated == 0 && creation_gated == 0 {
                    2
                } else {
                    0
                }
            );
            assert_eq!(r["funds"]["expenses"][6], i32::MIN);
            assert_eq!(r["funds"]["money"], json!([3647, 18140]));
        }
    }
}
#[test]
fn budget_mode_one_returns_before_absent_storage_and_category_validation() {
    let mut w = World::default();
    w.globals.insert(32, 1);
    let mut c = Application::new(i32::MIN, i32::MAX);
    assert_eq!(c.advance(&mut w).unwrap(), None);
    assert!(c.complete());
    assert!(w.budget.is_none());
    assert!(!w.globals.contains_key(&13));
    w.globals.insert(32, 0);
    let before = serde_json::to_value(&w).unwrap();
    assert!(Application::new(9, 5).advance(&mut w).is_err());
    assert_eq!(serde_json::to_value(w).unwrap(), before);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn direct_application_matches_219_previously_executed_native_budget_cases() {
    //Reuse original instruction results already collected for44eade->46d91c.
    //Only successful, non-check calls enter the direct application; rejected
    //affordability and check-only branches are deliberately not compared here.
    let report: Value =
        serde_json::from_str(include_str!("../../sims-vm/tests/fixtures/budget.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let mut count = 0;
    for row in report["cases"].as_array().unwrap() {
        let (c, r) = (&row["input"], &row["original"]);
        let flags = c["flags"].as_u64().unwrap();
        if !r["error"].is_null() || r["returned"] != 1 || flags & 1 != 0 {
            continue;
        }
        let value = c["value"].as_i64().unwrap() as i32;
        let mut world = World {
            budget: Some(State {
                balance: c["balance"].as_i64().unwrap() as i32,
                expenses: serde_json::from_value(c["expenses"].clone()).unwrap(),
                notifications: vec![],
            }),
            globals: BTreeMap::from([
                (13, 123),
                (14, 456),
                (32, c["mode"].as_i64().unwrap() as i16),
            ]),
            ..Default::default()
        };
        let mut call = Application::new(
            c["category"].as_i64().unwrap() as i32,
            if flags & 2 != 0 { -value } else { value },
        );
        let notice = call.advance(&mut world).unwrap();
        (call, world) = cold(&(call, world));
        let state = world.budget.as_ref().unwrap();
        assert_eq!(json!(state.balance), r["balance"], "{c}");
        assert_eq!(json!(state.expenses), r["expenses"], "{c}");
        assert_eq!(
            json!([world.globals[&13], world.globals[&14]]),
            r["money_words"],
            "{c}"
        );
        assert_eq!(
            json!(notice.map(|v| vec![json!([0xda, v])]).unwrap_or_default()),
            r["signals"],
            "{c}"
        );
        if let Some(value) = notice {
            call.acknowledge(value).unwrap();
        }
        assert!(call.complete());
        count += 1;
    }
    assert_eq!(count, 219);
}
#[test]
fn recursive_kind_setter_preserves_native_prefix_and_null_child_failure() {
    let (mut s, mut o) = (Store::default(), Objects::default());
    for id in 1..=2 {
        s.construct_command(id).unwrap();
        s.lists.insert(List::Objects(id), Vector::default());
        o.0.insert(id, object(12));
    }
    s.append(List::Objects(1), 2).unwrap();
    s.append(List::Objects(1), 0).unwrap();
    let mut call = kind::Call::new(1, 0xffffffff);
    assert!(
        call.advance(&s, &mut o, 10)
            .unwrap_err()
            .contains("null child")
    );
    assert_eq!(o.0[&1].kind, -1);
    assert_eq!(o.0[&2].kind, -1);
}
#[test]
fn move_reset_waits_for_recursive_kind_stores_before_appending_history() {
    use sims_sim::object_catalog::{
        definition::Catalog,
        move_tool::{Groups, cleanup, dispatch, hover},
    };
    struct Host;
    impl cleanup::View for Host {
        fn disposable(&self, _: u32) -> Result<bool, String> {
            panic!("unexpected held-object query")
        }
        fn script_id(&self, _: u32) -> Result<i16, String> {
            panic!("unexpected script-ID query")
        }
        fn clear_payload_cost(&mut self, _: u32) -> Result<(), String> {
            panic!("unexpected cost store")
        }
    }
    impl hover::View for Host {
        fn lookup(&self, _: i16) -> Result<Option<u32>, String> {
            panic!("unexpected hover lookup")
        }
        fn flags(&self, _: u32) -> Result<u32, String> {
            panic!("unexpected hover flags")
        }
    }
    let (mut store, mut objects) = (Store::default(), Objects::default());
    for id in [1, 2] {
        store.construct_command(id).unwrap();
        store.lists.insert(List::Objects(id), Vector::default());
        objects.0.insert(id, object(0));
    }
    store.append(List::Objects(1), 2).unwrap();
    let mut state = cleanup::State::constructor_fields(900);
    state.held = Some(777);
    state.payload = Some(1);
    let input = cleanup::Inputs {
        registry: 100,
        window: 0,
        key_function: 0,
        control_state: 0,
    };
    let mut parent = dispatch::Dispatch::cleanup(cleanup::Kind::Reset);
    let advance = |parent: &mut dispatch::Dispatch, state: &mut cleanup::State| {
        parent
            .advance(
                state,
                &mut Host,
                input,
                &Groups::default(),
                &Catalog::default(),
                0,
            )
            .unwrap()
    };
    assert_eq!(
        advance(&mut parent, &mut state),
        Some(dispatch::Service::External(cleanup::Request::PayloadKind {
            payload: 1,
            value: 3
        }))
    );
    let mut child = kind::Call::new(1, 3);
    assert!(child.finish(&mut parent).is_err());
    assert!(!child.advance(&store, &mut objects, 1).unwrap());
    (parent, state, child, store, objects) = cold(&(parent, state, child, store, objects));
    assert!(child.finish(&mut parent).is_err());
    assert!(child.advance(&store, &mut objects, 10).unwrap());
    child.finish(&mut parent).unwrap();
    assert_eq!(objects.0[&1].kind, 3);
    assert_eq!(objects.0[&2].kind, 3);
    assert_eq!(
        advance(&mut parent, &mut state),
        Some(dispatch::Service::External(cleanup::Request::Append {
            clipboard: 900,
            payload: 1
        }))
    );
}
#[test]
#[ignore = "private prepared inputs; does not execute original instructions"]
fn prepared_budget_and_kind_cases_restore_at_every_boundary() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-object-budget-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], false);
    assert_eq!(report["cases"].as_array().unwrap().len(), 192);
    for row in report["cases"].as_array().unwrap() {
        assert_eq!(
            run(&row["input"], false),
            run(&row["input"], true),
            "{}",
            row["input"]
        );
    }
}
#[test]
#[ignore = "requires guarded original46d91c/DA/48bcc2 corpus"]
fn budget_and_kind_match_original_instructions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/object-budget.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], true);
    assert_eq!(report["cases"].as_array().unwrap().len(), 192);
    for row in report["cases"].as_array().unwrap() {
        assert_eq!(
            run(&row["input"], true),
            row["original"],
            "{}",
            row["input"]
        );
    }
}
