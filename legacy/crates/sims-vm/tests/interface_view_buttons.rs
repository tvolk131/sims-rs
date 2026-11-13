use serde_json::{Value, json};
use sims_vm::{
    interface::view_control::buttons::*,
    ui_effect::widgets::{
        button::{AuxiliarySetter, Request as WidgetRequest},
        timers::*,
    },
};
use std::collections::BTreeMap;

fn fields() -> Fields {
    Fields {
        phase: 2,
        timer_registered: 1,
        alternate: 1,
    }
}
fn inputs() -> Inputs {
    Inputs {
        mode: 2,
        paused: 128,
        speed: 760,
        window_blocked: 0,
        timer_owner: 0x10004000,
        pause: 0x10006000,
        ultra: 0x10007000,
    }
}
fn merge(to: &mut Value, from: &Value) {
    if let (Some(a), Some(b)) = (to.as_object_mut(), from.as_object()) {
        for (k, v) in b {
            merge(
                a.get_mut(k).unwrap_or_else(|| panic!("unknown patch {k}")),
                v,
            );
        }
    } else {
        *to = from.clone();
    }
}
fn button_run(c: &Value, restore: bool) -> Value {
    let mut f: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
    let mut i: Inputs = serde_json::from_value(c["inputs"].clone()).unwrap();
    let kind: Kind = serde_json::from_value(c["call"].clone()).unwrap();
    let mut call = Call::new(0x10000000, kind);
    let mut events = vec![];
    for n in 0..20 {
        let Some(r) = call.advance(&mut f, i).unwrap() else {
            break;
        };
        call.validate().unwrap();
        let snapshot = json!({"fields":f,"inputs":i});
        events.push(json!({"request":r,"state":snapshot}));
        if restore {
            (call, f, i) =
                serde_json::from_slice(&serde_json::to_vec(&(&call, f, i)).unwrap()).unwrap();
            assert_eq!(call.advance(&mut f, i).unwrap(), Some(r));
            assert_eq!(json!({"fields":f,"inputs":i}), snapshot);
            let before = call.clone();
            assert!(
                call.acknowledge(Request::Selected {
                    widget: 0,
                    value: -123
                })
                .is_err()
            );
            assert_eq!(call, before);
        }
        if let Some(patch) = c["patches"].get(n.to_string()) {
            let mut v = json!({"fields":f,"inputs":i});
            merge(&mut v, patch);
            f = serde_json::from_value(v["fields"].clone()).unwrap();
            i = serde_json::from_value(v["inputs"].clone()).unwrap();
        }
        call.acknowledge(r).unwrap();
        if restore {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
    }
    assert!(call.complete());
    json!({"events":events,"state":{"fields":f,"inputs":i}})
}
fn auxiliary_run(c: &Value, restore: bool) -> Value {
    let mut state = c["initial"].as_i64().unwrap() as i32;
    let mut call = AuxiliarySetter::new(c["value"].as_i64().unwrap() as i32);
    let mut events = vec![];
    if let Some(r) = call.advance(&mut state) {
        assert_eq!(r, WidgetRequest::Invalidate);
        events.push(state);
        if restore {
            (call, state) =
                serde_json::from_slice(&serde_json::to_vec(&(&call, state)).unwrap()).unwrap();
            assert_eq!(call.advance(&mut state), Some(r.clone()));
        }
        if let Some(n) = c["mutation"].as_i64() {
            state = n as i32;
        }
        call.acknowledge(&r).unwrap();
    }
    if restore {
        call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
    }
    assert_eq!(call.advance(&mut state), None);
    assert!(call.complete());
    json!({"invalidations":events,"state":state})
}
fn timer_run(c: &Value, restore: bool) -> Value {
    let mut reg: Registry = serde_json::from_value(c["registry"].clone()).unwrap();
    let mut before_clock = None;
    if c["kind"] == "register" {
        let mut call = Registration::new(
            c["owner"].as_u64().unwrap() as u32,
            c["interval"].as_u64().unwrap() as u32,
            c["argument"].as_u64().unwrap() as u32,
        );
        assert!(call.advance(&mut reg).unwrap());
        before_clock = Some(reg.clone());
        if restore {
            (call, reg) =
                serde_json::from_slice(&serde_json::to_vec(&(&call, &reg)).unwrap()).unwrap();
            assert!(call.advance(&mut reg).unwrap());
            assert_eq!(Some(reg.clone()), before_clock);
        }
        if !c["patch"].is_null() {
            reg = serde_json::from_value(c["patch"].clone()).unwrap();
        }
        call.clock(c["clock"].as_u64().unwrap() as u32).unwrap();
        if restore {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
        assert!(!call.advance(&mut reg).unwrap());
        assert!(call.complete());
    } else {
        reg.remove(c["owner"].as_u64().unwrap() as u32).unwrap();
    }
    json!({"before_clock":before_clock,"registry":reg})
}
fn run(c: &Value, restore: bool) -> Value {
    match c["kind"].as_str().unwrap() {
        "buttons" => button_run(c, restore),
        "auxiliary" => auxiliary_run(c, restore),
        "register" | "remove" => timer_run(c, restore),
        _ => panic!("unknown case"),
    }
}
fn entry(owner: u32) -> Entry {
    Entry {
        owner,
        interval: 10,
        deadline: 20,
        argument: 30,
    }
}
fn checked(c: &Value) -> Value {
    let a = run(c, false);
    assert_eq!(a, run(c, true));
    a
}

#[test]
fn same_pause_state_still_cleans_up_and_enters_after_saved_callbacks() {
    let c = json!({"kind":"buttons","call":{"Dispatch":1},"fields":fields(),"inputs":inputs(),"patches":{}});
    let v = checked(&c);
    let requests: Vec<Request> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| serde_json::from_value(e["request"].clone()).unwrap())
        .collect();
    let i = inputs();
    assert_eq!(
        requests,
        vec![
            Request::Unregister {
                manager: i.timer_owner,
                view: 0x10000000
            },
            Request::Selected {
                widget: i.pause,
                value: 0
            },
            Request::Auxiliary {
                widget: i.pause,
                value: 0
            },
            Request::Register {
                manager: i.timer_owner,
                view: 0x10000000,
                interval: 500,
                argument: 0
            },
            Request::Selected {
                widget: i.pause,
                value: 1
            }
        ]
    );
    assert_eq!(v["events"][0]["state"]["fields"]["timer_registered"], 1);
    assert_eq!(v["events"][1]["state"]["fields"]["timer_registered"], 0);
    assert_eq!(v["events"][3]["state"]["fields"]["phase"], 2);
    assert_eq!(v["events"][3]["state"]["fields"]["timer_registered"], 0);
    assert_eq!(v["state"]["fields"], json!(fields()));
}
#[test]
fn transition_retains_target_but_rereads_widget_and_timer_owners() {
    let c = json!({"kind":"buttons","call":{"Transition":1},"fields":fields(),"inputs":inputs(),"patches":{
        "0":{"fields":{"timer_registered":128},"inputs":{"pause":11}},
        "1":{"fields":{"phase":99},"inputs":{"pause":12}},
        "2":{"inputs":{"pause":13}},"3":{"fields":{"phase":55},"inputs":{"pause":14}}}});
    let v = checked(&c);
    let e = &v["events"];
    assert_eq!(
        e[1]["request"],
        json!(Request::Selected {
            widget: 11,
            value: 0
        })
    );
    assert_eq!(
        e[2]["request"],
        json!(Request::Auxiliary {
            widget: 12,
            value: 0
        })
    );
    assert_eq!(
        e[3]["request"],
        json!(Request::Auxiliary {
            widget: 13,
            value: 1
        })
    );
    assert_eq!(e[3]["state"]["fields"]["phase"], 1);
    assert_eq!(
        e[4]["request"],
        json!(Request::Selected {
            widget: 14,
            value: 1
        })
    );
    assert_eq!(v["state"]["fields"]["phase"], 55);
    assert_eq!(v["state"]["fields"]["timer_registered"], 0);
}
#[test]
fn cleanup_rereads_speed_after_unregister_and_obeys_captured_old_state() {
    let mut f = fields();
    f.phase = 3;
    let c = json!({"kind":"buttons","call":{"Transition":3},"fields":f,"inputs":inputs(),"patches":{
        "0":{"fields":{"phase":0},"inputs":{"speed":994,"timer_owner":123}},
        "1":{"fields":{"timer_registered":128}}}});
    let v = checked(&c);
    assert_eq!(v["events"].as_array().unwrap().len(), 2);
    assert_eq!(
        v["events"][1]["request"],
        json!(Request::Register {
            manager: 123,
            view: 0x10000000,
            interval: 500,
            argument: 0
        })
    );
    assert_eq!(v["state"]["fields"]["timer_registered"], 1);
    assert_eq!(v["state"]["fields"]["phase"], 3);
    let c =
        json!({"kind":"buttons","call":{"Cleanup":1},"fields":f,"inputs":inputs(),"patches":{}});
    let v = checked(&c);
    assert_eq!(v["state"]["fields"], json!(f));
    assert_eq!(
        v["events"][0]["request"],
        json!(Request::Auxiliary {
            widget: inputs().pause,
            value: 0
        })
    );
}
#[test]
fn dispatcher_keeps_distinct_reason_and_mode_gates() {
    let mut f = fields();
    let mut i = inputs();
    i.paused = 0;
    i.window_blocked = 1;
    assert_eq!(decision(f, i, 0), Some(0)); // phase2 reason0 does not read window119
    assert_eq!(decision(f, i, 1), Some(3));
    f.phase = 1;
    assert_eq!(decision(f, i, 3), None);
    f.phase = 0;
    assert_eq!(decision(f, i, 3), Some(1));
    i.speed = 994;
    assert_eq!(decision(f, i, 2), Some(3)); // phase0 reason2 has no Ultra gate
    f.phase = 1;
    i.window_blocked = 0;
    assert_eq!(decision(f, i, 3), Some(0));
    f.phase = 3;
    assert_eq!(decision(f, i, 0), None);
    assert_eq!(decision(f, i, 2), Some(0));
    f.phase = -1;
    assert_eq!(decision(f, i, 1), None);
}
#[test]
fn auxiliary_setter_invalidates_before_write_and_rejects_wrong_acknowledgement() {
    let v = checked(&json!({"kind":"auxiliary","initial":3,"value":1,"mutation":99}));
    assert_eq!(v, json!({"invalidations":[3],"state":1}));
    let v = checked(&json!({"kind":"auxiliary","initial":1,"value":1,"mutation":99}));
    assert_eq!(v, json!({"invalidations":[],"state":1}));
    let mut call = AuxiliarySetter::new(7);
    let before = call.clone();
    assert!(call.acknowledge(&WidgetRequest::Invalidate).is_err());
    assert_eq!(call, before);
    let mut state = 0;
    assert_eq!(call.advance(&mut state), Some(WidgetRequest::Invalidate));
    let before = call.clone();
    assert!(call.acknowledge(&WidgetRequest::Changed).is_err());
    assert_eq!(call, before);
}
#[test]
fn registration_samples_clock_before_lookup_and_replaces_only_first_duplicate() {
    let reg = Registry {
        entries: vec![entry(1), entry(2)],
        first: 1,
        second: 2,
    };
    let patch = Registry {
        entries: vec![entry(1), entry(3), entry(3)],
        first: -1,
        second: 9,
    };
    let v = checked(
        &json!({"kind":"register","registry":reg,"owner":3,"interval":10,"argument":77,"clock":u32::MAX-4,"patch":patch}),
    );
    let got: Registry = serde_json::from_value(v["registry"].clone()).unwrap();
    assert_eq!(
        got.entries,
        vec![
            entry(1),
            Entry {
                owner: 3,
                interval: 10,
                deadline: 5,
                argument: 77
            },
            entry(3)
        ]
    );
    assert_eq!((got.first, got.second), (-1, 9));
    assert_eq!(v["before_clock"], json!(reg));
}
#[test]
fn timer_removal_adjusts_signed_indices_strictly_after_first_match() {
    for first in [-1, 0, 1, 2, 3, i32::MAX] {
        for second in [-1, 0, 1, 2, 3] {
            let mut r = Registry {
                entries: vec![entry(1), entry(2), entry(2)],
                first,
                second,
            };
            r.remove(2).unwrap();
            assert_eq!(r.entries, vec![entry(1), entry(2)]);
            assert_eq!(r.first, first - i32::from(first > 1));
            assert_eq!(r.second, second - i32::from(second > 1));
            let before = r.clone();
            r.remove(77).unwrap();
            assert_eq!(r, before);
        }
    }
}
#[test]
fn button_child_requests_can_run_real_registry_and_widget_setters() {
    let mut f = fields();
    let i = inputs();
    let mut call = Call::new(7, Kind::Transition(2));
    let mut registry = Registry {
        entries: vec![entry(7)],
        first: 1,
        second: 1,
    };
    let mut states = BTreeMap::from([(i.pause, (1, 1))]);
    while let Some(r) = call.advance(&mut f, i).unwrap() {
        match r {
            Request::Unregister { view, .. } => registry.remove(view).unwrap(),
            Request::Register {
                view,
                interval,
                argument,
                ..
            } => {
                let mut c = Registration::new(view, interval, argument);
                assert!(c.advance(&mut registry).unwrap());
                c.clock(1000).unwrap();
                assert!(!c.advance(&mut registry).unwrap());
            }
            Request::Auxiliary { widget, value } => {
                let mut c = AuxiliarySetter::new(value);
                let s = &mut states.get_mut(&widget).unwrap().1;
                if let Some(r) = c.advance(s) {
                    c.acknowledge(&r).unwrap();
                    assert_eq!(c.advance(s), None);
                }
            }
            Request::Selected { widget, value } => {
                let mut c = sims_vm::ui_effect::widgets::button::Setter::new(value);
                let s = &mut states.get_mut(&widget).unwrap().0;
                while let Some(r) = c.advance(s).unwrap() {
                    let reply = if r == WidgetRequest::Parent {
                        sims_vm::ui_effect::widgets::button::Reply::Parent(None)
                    } else {
                        sims_vm::ui_effect::widgets::button::Reply::Done
                    };
                    c.acknowledge(&r, reply).unwrap();
                }
            }
        }
        call.acknowledge(r).unwrap();
    }
    assert!(call.complete());
    assert_eq!(f.phase, 2);
    assert_eq!(f.timer_registered, 1);
    assert_eq!(
        registry.entries,
        vec![Entry {
            owner: 7,
            interval: 500,
            deadline: 1500,
            argument: 0
        }]
    );
    assert_eq!(states[&i.pause], (1, 0));
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
#[ignore = "private prepared cases; Rust continuations, no original execution"]
fn prepared_view_button_cases_restore_every_callback() {
    let v = corpus("native-view-buttons-inputs.json");
    assert_eq!(v["native_execution"], false);
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case{n}: {c}");
    }
}
#[test]
#[ignore = "guarded original button/auxiliary/timer caller corpus; children scoped in harness"]
fn view_buttons_and_timer_registration_match_original() {
    let v = corpus("research/sims-exe/oracle/view-buttons.json");
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
