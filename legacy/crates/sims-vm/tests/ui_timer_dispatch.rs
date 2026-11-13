use serde_json::{Value, json};
use sims_vm::{
    interface::view_control::{
        buttons,
        timer::{Callback, WidgetState},
    },
    ui_effect::widgets::{
        button::{AuxiliarySetter, Reply as ButtonReply, Request as ButtonRequest, Setter},
        timers::{
            self, Entry, Registration, Registry,
            dispatch::{Flags, Pass, Reply, Request},
        },
    },
};
use std::collections::BTreeMap;

fn entry(owner: u32, deadline: u32) -> Entry {
    Entry {
        owner,
        deadline,
        interval: 10,
        argument: owner + 100,
    }
}
fn registry(entries: Vec<Entry>) -> Registry {
    Registry {
        entries,
        first: -1,
        second: -1,
    }
}
fn input() -> buttons::Inputs {
    buttons::Inputs {
        mode: 2,
        paused: 1,
        speed: 760,
        window_blocked: 0,
        timer_owner: 99,
        pause: 11,
        ultra: 12,
    }
}
fn merge(to: &mut Value, from: &Value) {
    if let (Some(a), Some(b)) = (to.as_object_mut(), from.as_object()) {
        for (k, v) in b {
            merge(
                a.get_mut(k)
                    .unwrap_or_else(|| panic!("unknown mutation {k}")),
                v,
            );
        }
    } else {
        *to = from.clone();
    }
}
fn dispatch_run(c: &Value, restore: bool) -> Value {
    let mut reg: Registry = serde_json::from_value(c["registry"].clone()).unwrap();
    let mut flags: Flags = serde_json::from_value(c["flags"].clone()).unwrap();
    let mut owners: Vec<u32> = serde_json::from_value(c["owners"].clone()).unwrap();
    let mut special = c["special"].as_u64().unwrap() as u32;
    let mut call = Pass::default();
    let mut events = vec![];
    for n in 0..2000 {
        let Some(r) = call.advance(&mut reg, &mut flags).unwrap() else {
            break;
        };
        let snapshot = json!({"registry":reg,"flags":flags,"owners":owners,"special":special});
        events.push(json!({"request":r,"state":snapshot}));
        if restore {
            (call, reg, flags, owners, special) = serde_json::from_slice(
                &serde_json::to_vec(&(&call, &reg, flags, &owners, special)).unwrap(),
            )
            .unwrap();
            assert_eq!(call.advance(&mut reg, &mut flags).unwrap(), Some(r));
            assert_eq!(
                json!({"registry":reg,"flags":flags,"owners":owners,"special":special}),
                snapshot
            );
            let before = call.clone();
            assert!(
                call.acknowledge(Request::Registered(u32::MAX), Reply::Done)
                    .is_err()
            );
            assert_eq!(call, before);
        }
        let reply = match r {
            Request::PointerPosition => {
                Reply::Point(serde_json::from_value(c["point"].clone()).unwrap())
            }
            Request::PublishPointer { .. } | Request::Dispatch { .. } => Reply::Done,
            Request::Clock => Reply::Clock(c["now"].as_u64().unwrap() as u32),
            Request::Registered(owner) => {
                Reply::Registered(timers::registered(owner, special, &owners))
            }
        };
        if let Some(actions) = c["actions"].get(n.to_string()) {
            for a in actions.as_array().unwrap() {
                if let Some(id) = a["remove"].as_u64() {
                    reg.remove(id as u32).unwrap();
                }
                if !a["register"].is_null() {
                    reg.register(serde_json::from_value(a["register"].clone()).unwrap())
                        .unwrap();
                }
            }
        }
        if let Some(p) = c["patches"].get(n.to_string()) {
            let mut v = json!({"registry":reg,"flags":flags,"owners":owners,"special":special});
            merge(&mut v, p);
            reg = serde_json::from_value(v["registry"].clone()).unwrap();
            flags = serde_json::from_value(v["flags"].clone()).unwrap();
            owners = serde_json::from_value(v["owners"].clone()).unwrap();
            special = v["special"].as_u64().unwrap() as u32;
        }
        call.acknowledge(r, reply).unwrap();
        if restore {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
    }
    assert!(call.complete());
    json!({"events":events,"state":{"registry":reg,"flags":flags,"owners":owners,"special":special}})
}
fn callback_run(c: &Value, restore: bool) -> Value {
    let mut fields: buttons::Fields = serde_json::from_value(c["fields"].clone()).unwrap();
    let mut input: buttons::Inputs = serde_json::from_value(c["inputs"].clone()).unwrap();
    let mut widgets: BTreeMap<u32, WidgetState> =
        serde_json::from_value(c["widgets"].clone()).unwrap();
    let mut call = Callback::new(c["view"].as_u64().unwrap_or(7) as u32);
    let mut events = vec![];
    for n in 0..5 {
        let Some(r) = call.advance(&fields, input, &widgets).unwrap() else {
            break;
        };
        let snapshot = json!({"fields":fields,"inputs":input,"widgets":widgets});
        events.push(json!({"request":r,"state":snapshot}));
        if restore {
            (call, fields, input, widgets) = serde_json::from_slice(
                &serde_json::to_vec(&(&call, fields, input, &widgets)).unwrap(),
            )
            .unwrap();
            assert_eq!(call.advance(&fields, input, &widgets).unwrap(), Some(r));
        }
        if let Some(p) = c["patches"].get(n.to_string()) {
            let mut v = json!({"fields":fields,"inputs":input,"widgets":widgets});
            merge(&mut v, p);
            fields = serde_json::from_value(v["fields"].clone()).unwrap();
            input = serde_json::from_value(v["inputs"].clone()).unwrap();
            widgets = serde_json::from_value(v["widgets"].clone()).unwrap();
        }
        call.acknowledge(r).unwrap();
        if restore {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
    }
    assert!(call.complete());
    json!({"events":events,"state":{"fields":fields,"inputs":input,"widgets":widgets}})
}
fn run(c: &Value, restore: bool) -> Value {
    match c["kind"].as_str().unwrap() {
        "dispatch" => dispatch_run(c, restore),
        "callback" => callback_run(c, restore),
        "sort" => {
            let mut v: Vec<Entry> = serde_json::from_value(c["entries"].clone()).unwrap();
            timers::sort_entries(&mut v);
            json!(v)
        }
        "membership" => json!(timers::registered(
            c["owner"].as_u64().unwrap() as u32,
            c["special"].as_u64().unwrap() as u32,
            &serde_json::from_value::<Vec<u32>>(c["owners"].clone()).unwrap()
        )),
        _ => panic!("unknown case"),
    }
}
fn case(entries: Vec<Entry>, now: u32) -> Value {
    json!({"kind":"dispatch","registry":registry(entries),"flags":{"running":0,"pointer_pending":0},"now":now,
        "owners":[1,2,3,7],"special":77,"point":[2,3],"actions":{},"patches":{}})
}
fn checked(c: &Value) -> Value {
    let a = run(c, false);
    assert_eq!(a, run(c, true));
    a
}
fn dispatches(v: &Value) -> Vec<u32> {
    v["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["request"]["Dispatch"]["owner"].as_u64().map(|n| n as u32))
        .collect()
}

#[test]
fn timer_pass_publishes_pointer_before_sort_clock_and_deadline_before_membership() {
    let mut c = case(vec![entry(2, 20), entry(1, 10)], 10);
    c["flags"] = json!({"running":128,"pointer_pending":1});
    c["point"] = json!([0x12345678u32, 0xabcdef01u32]);
    c["patches"] = json!({"1":{"flags":{"pointer_pending":128}}});
    let v = checked(&c);
    let e = &v["events"];
    assert_eq!(e[0]["request"], json!("PointerPosition"));
    assert_eq!(
        e[1]["request"],
        json!(Request::PublishPointer {
            message: 13,
            argument: 0,
            packed: 0x12345678 | 0xef010000
        })
    );
    assert_eq!(e[1]["state"]["flags"]["running"], 128);
    assert_eq!(e[2]["request"], json!("Clock"));
    assert_eq!(
        e[2]["state"]["flags"],
        json!({"running":1,"pointer_pending":0})
    );
    assert_eq!(e[2]["state"]["registry"]["entries"][0]["owner"], 1);
    assert_eq!(e[3]["request"], json!(Request::Registered(1)));
    assert_eq!(e[3]["state"]["registry"]["entries"][0]["deadline"], 20);
    assert_eq!(dispatches(&v), vec![1]);
    assert_eq!(v["state"]["flags"]["running"], 0);
}
#[test]
fn signed_deadlines_sort_and_partition_ties_follow_original_compiler_algorithm() {
    let mut v = vec![
        entry(1, 0),
        entry(2, u32::MAX),
        entry(3, 0x80000000),
        entry(4, 0x7fffffff),
    ];
    timers::sort_entries(&mut v);
    assert_eq!(
        v.iter().map(|e| e.owner).collect::<Vec<_>>(),
        vec![3, 2, 1, 4]
    );
    for n in [16, 17, 32] {
        let mut v: Vec<_> = (0..n).map(|i| entry(i, 10)).collect();
        timers::sort_entries(&mut v);
        let expected: Vec<_> = if n == 16 {
            (0..n).collect()
        } else {
            (0..n).rev().collect()
        };
        assert_eq!(v.iter().map(|e| e.owner).collect::<Vec<_>>(), expected);
    }
    let v = checked(&case(vec![entry(1, 0xfffffff0)], 0));
    assert_eq!(dispatches(&v), vec![1]);
    assert_eq!(
        v["state"]["registry"]["entries"][0]["deadline"],
        0xfffffffa_u32
    );
    let v = checked(&case(vec![entry(1, 0)], u32::MAX));
    assert!(dispatches(&v).is_empty());
}
#[test]
fn timer_pass_retains_clock_bound_and_slot_but_rereads_event_fields() {
    let mut c = case(vec![entry(1, 10), entry(2, 10)], 10);
    c["patches"] = json!({"1":{"registry":{"entries":[entry(3,20),entry(2,10)]},"owners":[]}});
    let v = checked(&c);
    assert_eq!(dispatches(&v), vec![3]); // presence reply for1, current slot fields for3
    let mut c = case(vec![entry(1, 10), entry(2, 10)], 10);
    c["registry"]["first"] = json!(1);
    c["registry"]["second"] = json!(2);
    let v = checked(&c);
    assert_eq!(dispatches(&v), vec![2]);
    let mut c = case(vec![entry(1, 10)], 10);
    c["actions"] = json!({"2":[{"register":entry(2,0)}]});
    let v = checked(&c);
    assert_eq!(dispatches(&v), vec![1]);
    assert_eq!(
        v["state"]["registry"]["entries"].as_array().unwrap().len(),
        2
    );
}
#[test]
fn self_rearming_callback_preserves_native_skip_and_next_pass_reaches_shifted_entry() {
    let mut c = case(vec![entry(1, 10), entry(2, 10)], 10);
    c["actions"] = json!({"2":[{"remove":1},{"register":entry(1,500)}]});
    let v = checked(&c);
    assert_eq!(dispatches(&v), vec![1]);
    let mut next = case(vec![], 10);
    next["registry"] = v["state"]["registry"].clone();
    assert_eq!(dispatches(&checked(&next)), vec![2]);
}
#[test]
fn empty_and_absent_owner_paths_still_sample_clock_and_advance_deadlines() {
    let v = checked(&case(vec![], 10));
    assert_eq!(v["events"].as_array().unwrap().len(), 1);
    assert_eq!(v["events"][0]["request"], json!("Clock"));
    let v = checked(&case(vec![entry(99, 10), entry(0, 10), entry(77, 10)], 10));
    assert_eq!(dispatches(&v), vec![77]);
    assert!(
        v["state"]["registry"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["deadline"] == 20)
    );
    assert!(!timers::registered(0, 0, &[0]));
    assert!(timers::registered(7, 7, &[]));
    assert!(timers::registered(7, 0, &[7, 7]));
}
#[test]
fn view_timer_rearms_for_every_phase_and_keeps_registration_flag_untouched() {
    for phase in [-1, 0, 1, 2, 3, 4] {
        for value in [-1, 0, 1] {
            let c = json!({"kind":"callback","fields":{"phase":phase,"timer_registered":128,"alternate":1},"inputs":input(),
            "widgets":{"11":{"selected":5,"auxiliary":value},"12":{"selected":value,"auxiliary":7}},"patches":{}});
            let v = checked(&c);
            let e = v["events"].as_array().unwrap();
            assert_eq!(
                e[0]["request"],
                json!(buttons::Request::Unregister {
                    manager: 99,
                    view: 7
                })
            );
            assert_eq!(
                e.last().unwrap()["request"],
                json!(buttons::Request::Register {
                    manager: 99,
                    view: 7,
                    interval: 500,
                    argument: 0
                })
            );
            if phase == 2 {
                assert_eq!(
                    e[1]["request"],
                    json!(buttons::Request::Auxiliary {
                        widget: 11,
                        value: i32::from(value == 0)
                    })
                );
            }
            if phase == 3 {
                assert_eq!(
                    e[1]["request"],
                    json!(buttons::Request::Selected {
                        widget: 12,
                        value: i32::from(value == 0)
                    })
                );
            }
            assert_eq!(v["state"]["fields"]["timer_registered"], 128);
        }
    }
}
#[test]
fn view_timer_rereads_phase_speed_manager_and_honors_distinct_null_gates() {
    let mut c = json!({"kind":"callback","fields":{"phase":3,"timer_registered":0,"alternate":0},"inputs":input(),
        "widgets":{"11":{"selected":5,"auxiliary":0},"12":{"selected":0,"auxiliary":7}},
        "patches":{"0":{"fields":{"phase":2}},"1":{"inputs":{"timer_owner":88}}}});
    let v = checked(&c);
    assert_eq!(
        v["events"][1]["request"],
        json!(buttons::Request::Auxiliary {
            widget: 11,
            value: 1
        })
    );
    assert_eq!(v["events"][2]["request"]["Register"]["manager"], 88);
    c["patches"] = json!({"0":{"inputs":{"speed":994,"ultra":0}}});
    assert_eq!(checked(&c)["events"].as_array().unwrap().len(), 2);
    c["fields"]["phase"] = json!(2);
    c["patches"] = json!({});
    c["inputs"]["pause"] = json!(0);
    assert_eq!(checked(&c)["events"].as_array().unwrap().len(), 2);
}
#[test]
fn timer_pass_and_view_callback_compose_with_real_registry_and_widget_services() {
    let fields = buttons::Fields {
        phase: 2,
        timer_registered: 1,
        alternate: 0,
    };
    let input = input();
    let mut widgets = BTreeMap::from([(
        11,
        WidgetState {
            selected: 1,
            auxiliary: 0,
        },
    )]);
    let mut reg = registry(vec![entry(7, 0), entry(1, 0)]);
    let mut flags = Flags {
        running: 0,
        pointer_pending: 0,
    };
    let mut call = Pass::default();
    let mut seen = vec![];
    while let Some(r) = call.advance(&mut reg, &mut flags).unwrap() {
        let reply = match r {
            Request::Clock => Reply::Clock(100),
            Request::Registered(id) => Reply::Registered(timers::registered(id, 0, &[7, 1])),
            Request::Dispatch {
                owner: 7,
                message: 26,
                ..
            } => {
                seen.push(7);
                let mut cb = Callback::new(7);
                while let Some(r) = cb.advance(&fields, input, &widgets).unwrap() {
                    match r {
                        buttons::Request::Unregister { view, .. } => reg.remove(view).unwrap(),
                        buttons::Request::Register {
                            view,
                            interval,
                            argument,
                            ..
                        } => {
                            let mut registration = Registration::new(view, interval, argument);
                            assert!(registration.advance(&mut reg).unwrap());
                            registration.clock(101).unwrap();
                            assert!(!registration.advance(&mut reg).unwrap());
                        }
                        buttons::Request::Auxiliary { widget, value } => {
                            let state = &mut widgets.get_mut(&widget).unwrap().auxiliary;
                            let mut setter = AuxiliarySetter::new(value);
                            if let Some(r) = setter.advance(state) {
                                setter.acknowledge(&r).unwrap();
                                assert_eq!(setter.advance(state), None);
                            }
                        }
                        buttons::Request::Selected { widget, value } => {
                            let state = &mut widgets.get_mut(&widget).unwrap().selected;
                            let mut setter = Setter::new(value);
                            while let Some(r) = setter.advance(state).unwrap() {
                                setter
                                    .acknowledge(
                                        &r,
                                        if r == ButtonRequest::Parent {
                                            ButtonReply::Parent(None)
                                        } else {
                                            ButtonReply::Done
                                        },
                                    )
                                    .unwrap();
                            }
                        }
                    }
                    cb.acknowledge(r).unwrap();
                }
                assert!(cb.complete());
                Reply::Done
            }
            Request::Dispatch { owner, .. } => {
                seen.push(owner);
                Reply::Done
            }
            _ => panic!("unexpected service"),
        };
        call.acknowledge(r, reply).unwrap();
    }
    assert_eq!(seen, vec![7]);
    assert_eq!(widgets[&11].auxiliary, 1);
    assert_eq!(
        reg.entries,
        vec![
            entry(1, 0),
            Entry {
                owner: 7,
                deadline: 601,
                interval: 500,
                argument: 0
            }
        ]
    );
    assert_eq!((reg.first, reg.second), (-1, -1));
    assert_eq!(fields.timer_registered, 1);
}
#[test]
fn malformed_saved_timer_reply_is_rejected_before_mutating_shared_state() {
    let mut reg = registry(vec![entry(1, 0)]);
    let mut flags = Flags {
        running: 0,
        pointer_pending: 0,
    };
    let mut pass = Pass::default();
    let r = pass.advance(&mut reg, &mut flags).unwrap().unwrap();
    assert_eq!(r, Request::Clock);
    let before = pass.clone();
    assert!(pass.acknowledge(r, Reply::Done).is_err());
    assert_eq!(pass, before);
    pass.acknowledge(r, Reply::Clock(0)).unwrap();
    let mut v = serde_json::to_value(&pass).unwrap();
    v["answer"] = json!("Done");
    let mut bad: Pass = serde_json::from_value(v).unwrap();
    let before = (bad.clone(), reg.clone(), flags);
    assert!(bad.advance(&mut reg, &mut flags).is_err());
    assert_eq!((bad, reg, flags), before);
}
fn corpus(path: &str) -> Value {
    serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../cache")
                .join(path),
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
#[ignore = "private Rust continuation cases; no original execution"]
fn prepared_ui_timer_cases_restore_every_callback() {
    let v = corpus("native-ui-timer-dispatch-inputs.json");
    assert_eq!(v["native_execution"], false);
    assert_eq!(v["cases"].as_array().unwrap().len(), 756);
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case{n}: {c}");
    }
}
#[test]
#[ignore = "guarded native timer/sort/membership/ViewControl callback corpus"]
fn ui_timer_dispatch_matches_original_callers_and_sort() {
    let v = corpus("research/sims-exe/oracle/ui-timer-dispatch.json");
    assert_eq!(v["cases"].as_array().unwrap().len(), 756);
    assert_eq!(
        v["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        v["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            run(&c["input"], true),
            c["original"],
            "case{n}: {}",
            c["input"]
        );
    }
}
