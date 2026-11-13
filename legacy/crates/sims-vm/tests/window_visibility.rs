use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::ui_effect::widgets::{
    timers,
    visibility::Call,
    window::{self, Fields, Function, Invocation, Manager, Reply, Request, State},
};
use std::collections::BTreeMap;
fn saved<T: Serialize + DeserializeOwned>(v: T) -> T {
    serde_json::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap()
}
fn merge(to: &mut Value, p: &Value) {
    if let Some(o) = p.as_object() {
        for (k, v) in o {
            if v.is_object() {
                merge(&mut to[k], v)
            } else {
                to[k] = v.clone()
            }
        }
    }
}
fn state() -> State {
    State {
        global_manager: 9,
        windows: (1..=5)
            .map(|id| {
                (
                    id,
                    Fields {
                        parent: if id == 1 { 2 } else { 0 },
                        dirty: 0,
                        flags: 0,
                    },
                )
            })
            .collect(),
        managers: BTreeMap::from([
            (
                9,
                Manager {
                    focus: 1,
                    root: 3,
                    special: 0,
                    registered: vec![1, 2, 3, 4, 5],
                },
            ),
            (
                10,
                Manager {
                    focus: 2,
                    root: 4,
                    special: 0,
                    registered: vec![1, 2, 3, 4, 5],
                },
            ),
        ]),
    }
}
fn supplied(s: &State, r: &Request) -> Reply {
    match r {
        Request::Virtual(c) => match c.slot {
            0x18 => Reply::Word(window::parent(s.window(c.window).unwrap())),
            0x8c => Reply::Word(window::flags(s.window(c.window).unwrap(), c.arguments[0])),
            0x90 => Reply::Word(s.window(c.window).unwrap().flags),
            0x168 | 0x174 => Reply::Done,
            _ => panic!("unexpected virtual call"),
        },
        Request::Registered { manager, window } => {
            let m = s.manager(*manager).unwrap();
            Reply::Registered(timers::registered(*window, m.special, &m.registered))
        }
        Request::TakeFocus(_) => Reply::Word(s.manager(s.global_manager).unwrap().focus),
        Request::SwitchFocus { manager, .. } => Reply::Word(s.manager(*manager).unwrap().focus),
        Request::Event { .. } => Reply::Done,
    }
}
fn run(c: &Value, restore: bool) -> Value {
    let mut s: State = serde_json::from_value(c["state"].clone()).unwrap();
    let owner = c["owner"].as_u64().unwrap() as u32;
    let mut events = vec![];
    let f = &c["function"];
    let result = if let Some(words) = f["FocusEvent"]["words"].as_array() {
        let words: [u32; 4] = words
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let event = window::focus_event(owner, words).unwrap();
        Some(u32::from(window::accept_focus(event.arguments[0])))
    } else if let Some(mask) = f["QueryFlags"]["mask"].as_u64() {
        Some(window::flags(s.window(owner).unwrap(), mask as u32))
    } else if f == "Parent" {
        Some(window::parent(s.window(owner).unwrap()))
    } else if f == "Invalidate" {
        window::invalidate(s.windows.get_mut(&owner).unwrap());
        None
    } else {
        let mut call = Invocation::new(owner, serde_json::from_value(f.clone()).unwrap());
        while let Some(r) = call.advance(&mut s).unwrap() {
            if restore {
                (call, s) = saved((call, s));
            }
            let n = events.len().to_string();
            let reply = c["replies"]
                .get(&n)
                .cloned()
                .map(|r| serde_json::from_value(r).unwrap())
                .unwrap_or_else(|| supplied(&s, &r));
            let mut x = json!(s);
            events.push(json!({"request":r,"state":x}));
            merge(&mut x, &c["patches"][&n]);
            s = serde_json::from_value(x).unwrap();
            if restore {
                (call, s) = saved((call, s));
            }
            call.acknowledge(&r, reply).unwrap();
            if restore {
                call = saved(call);
            }
        }
        assert!(call.complete());
        call.result()
    };
    json!({"events":events,"state":s,"result":result})
}
fn case(owner: u32, function: Function) -> Value {
    json!({"owner":owner,"function":function,"state":state(),"patches":{},"replies":{}})
}
fn checked(c: &Value) -> Value {
    let a = run(c, false);
    assert_eq!(a, run(c, true));
    a
}
fn requests(v: &Value) -> Vec<Request> {
    v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| serde_json::from_value(e["request"].clone()).unwrap())
        .collect()
}
fn virt(window: u32, slot: u16, args: &[u32]) -> Request {
    Request::Virtual(Call::new(window, slot, args))
}
#[test]
fn flag_write_uses_any_overlap_exact_masks_and_low_byte_value() {
    let mut c = case(
        1,
        Function::Flags {
            mask: 129,
            value: 1,
        },
    );
    c["state"]["windows"]["1"]["flags"] = json!(1);
    let v = checked(&c);
    assert_eq!(v["result"], 129);
    assert!(requests(&v).is_empty());
    c = case(
        1,
        Function::Flags {
            mask: 1,
            value: 256,
        },
    );
    c["state"]["windows"]["1"]["flags"] = json!(129);
    let v = checked(&c);
    let r = requests(&v);
    assert_eq!(v["events"][0]["state"]["windows"]["1"]["flags"], 128);
    assert_eq!(r[0], virt(1, 0x8c, &[128]));
    assert_eq!(r[1], virt(1, 0x174, &[256, 0]));
    assert_eq!(v["result"], 128);
    c = case(
        1,
        Function::Flags {
            mask: 128,
            value: 1,
        },
    );
    c["replies"] = json!({"0":{"Word":256}});
    assert_eq!(
        requests(&checked(&c)),
        vec![virt(1, 0x8c, &[1]), virt(1, 0x174, &[0, 0])]
    );
    c = case(
        1,
        Function::Flags {
            mask: 0,
            value: u32::MAX,
        },
    );
    assert!(requests(&checked(&c)).is_empty());
}
#[test]
fn flag_setter_preserves_argument_and_rereads_second_parent_and_final_flags() {
    let mut c = case(
        1,
        Function::Flags {
            mask: 1,
            value: 257,
        },
    );
    c["state"]["windows"]["1"]["flags"] = json!(128);
    c["patches"] = json!({"0":{"windows":{"1":{"flags":256}}},"3":{"windows":{"1":{"parent":3}}},"5":{"windows":{"1":{"flags":0xabcdef01u32}}}});
    let v = checked(&c);
    assert_eq!(
        requests(&v),
        vec![
            virt(1, 0x8c, &[128]),
            virt(1, 0x174, &[257, 0]),
            virt(1, 0x168, &[]),
            virt(1, 0x18, &[]),
            virt(1, 0x18, &[]),
            virt(3, 0x168, &[])
        ]
    );
    assert_eq!(v["result"], 0xabcdef01u32);
}
#[test]
fn hide_rereads_focus_manager_and_parent_then_uses_fresh_root_when_parent_absent() {
    let mut c = case(1, Function::Hide);
    c["patches"] = json!({"0":{"global_manager":10,"managers":{"10":{"focus":1}}},"1":{"windows":{"1":{"parent":3}}}});
    assert_eq!(
        requests(&checked(&c)),
        vec![
            virt(1, 0x90, &[1, 0]),
            virt(1, 0x18, &[]),
            virt(1, 0x18, &[]),
            Request::TakeFocus(3)
        ]
    );
    c = case(1, Function::Hide);
    c["replies"] = json!({"1":{"Word":0}});
    c["patches"] = json!({"1":{"global_manager":10}});
    assert_eq!(
        requests(&checked(&c)),
        vec![
            virt(1, 0x90, &[1, 0]),
            virt(1, 0x18, &[]),
            Request::TakeFocus(4)
        ]
    );
    c = case(1, Function::Hide);
    c["patches"] = json!({"0":{"managers":{"9":{"focus":2}}}});
    assert_eq!(requests(&checked(&c)), vec![virt(1, 0x90, &[1, 0])]);
}
#[test]
fn take_focus_checks_reply_low_byte_and_uses_current_manager() {
    let mut c = case(1, Function::TakeFocus);
    c["replies"] = json!({"0":{"Word":256},"1":{"Word":5}});
    c["patches"] = json!({"0":{"global_manager":10}});
    let v = checked(&c);
    assert_eq!(
        requests(&v),
        vec![
            virt(1, 0x8c, &[0x2000]),
            Request::SwitchFocus {
                manager: 10,
                window: 1
            }
        ]
    );
    assert_eq!(v["result"], 5);
    c["replies"]["0"] = json!({"Word":128});
    let v = checked(&c);
    assert_eq!(requests(&v), vec![virt(1, 0x8c, &[0x2000])]);
    assert_eq!(v["result"], 2);
}
#[test]
fn focus_switch_preserves_old_new_notifications_and_shared_callback_writes() {
    let mut c = case(9, Function::SwitchFocus { target: 2 });
    c["patches"] = json!({
        "0":{"managers":{"9":{"focus":3}}},"1":{"managers":{"9":{"focus":3}}},
        "2":{"managers":{"9":{"focus":3}}},"3":{"managers":{"9":{"focus":4}}},"4":{"managers":{"9":{"focus":5}}}});
    let v = checked(&c);
    let r = requests(&v);
    assert_eq!(
        r,
        vec![
            Request::Registered {
                manager: 9,
                window: 1
            },
            Request::Event {
                window: 1,
                words: [20, 2, 0, 0]
            },
            Request::Registered {
                manager: 9,
                window: 2
            },
            Request::Event {
                window: 2,
                words: [21, 1, 0, 0]
            },
            virt(4, 0x18, &[])
        ]
    );
    assert_eq!(v["events"][1]["state"]["managers"]["9"]["focus"], 3);
    assert_eq!(v["events"][2]["state"]["managers"]["9"]["focus"], 2);
    assert_eq!(v["result"], 5);
    let v = checked(&case(9, Function::SwitchFocus { target: 1 }));
    assert_eq!(requests(&v).len(), 5);
    assert_eq!(
        requests(&v)[1],
        Request::Event {
            window: 1,
            words: [20, 1, 0, 0]
        }
    );
    let mut c = case(9, Function::SwitchFocus { target: 2 });
    c["state"]["managers"]["9"]["registered"] = json!([]);
    assert_eq!(
        requests(&checked(&c)),
        vec![
            Request::Registered {
                manager: 9,
                window: 1
            },
            Request::Registered {
                manager: 9,
                window: 2
            },
            virt(2, 0x18, &[])
        ]
    );
    assert_eq!(
        requests(&checked(&case(9, Function::SwitchFocus { target: 0 }))).len(),
        2
    );
}
fn compose(
    mut call: Invocation,
    s: &mut State,
    events: &mut Vec<Request>,
    depth: usize,
) -> Option<u32> {
    assert!(depth < 10);
    while let Some(r) = call.advance(s).unwrap() {
        call = saved(call);
        *s = saved(s.clone());
        let reply = match &r {
            Request::Virtual(c) if c.slot == 0x90 => Reply::Word(
                compose(
                    Invocation::new(
                        c.window,
                        Function::Flags {
                            mask: c.arguments[0],
                            value: c.arguments[1],
                        },
                    ),
                    s,
                    events,
                    depth + 1,
                )
                .unwrap(),
            ),
            Request::Virtual(c) if c.slot == 0x168 => {
                window::invalidate(s.windows.get_mut(&c.window).unwrap());
                Reply::Done
            }
            Request::Virtual(c) if c.slot == 0x174 => {
                events.push(r.clone());
                Reply::Done
            } // explicit supplied surface/resource body
            Request::SwitchFocus { manager, window } => Reply::Word(
                compose(
                    Invocation::new(*manager, Function::SwitchFocus { target: *window }),
                    s,
                    events,
                    depth + 1,
                )
                .unwrap(),
            ),
            Request::TakeFocus(w) => Reply::Word(
                compose(
                    Invocation::new(*w, Function::TakeFocus),
                    s,
                    events,
                    depth + 1,
                )
                .unwrap(),
            ),
            Request::Event { window, words } => {
                let event = window::focus_event(*window, *words).unwrap();
                assert_eq!(window::accept_focus(event.arguments[0]), 1);
                events.push(r.clone());
                Reply::Done
            }
            _ => supplied(s, &r),
        };
        call = saved(call);
        call.acknowledge(&r, reply).unwrap();
    }
    assert!(call.complete());
    call.result()
}
#[test]
fn hide_composes_real_flag_getter_invalidation_membership_and_focus_kernels() {
    let mut s = state();
    s.windows.get_mut(&1).unwrap().flags = 1;
    let mut events = vec![];
    compose(Invocation::new(1, Function::Hide), &mut s, &mut events, 0);
    assert_eq!(s.windows[&1].flags, 0);
    assert_eq!(s.windows[&1].dirty, 1);
    assert_eq!(s.windows[&2].dirty, 1);
    assert_eq!(s.managers[&9].focus, 2);
    assert_eq!(
        events,
        vec![
            Request::Event {
                window: 1,
                words: [20, 2, 0, 0]
            },
            Request::Event {
                window: 2,
                words: [21, 1, 0, 0]
            }
        ]
    );
    events.clear();
    compose(Invocation::new(1, Function::Show), &mut s, &mut events, 0);
    assert_eq!(s.windows[&1].flags, 1);
    assert_eq!(s.managers[&9].focus, 2);
    assert!(events.is_empty());
}
#[test]
fn corrupt_window_continuations_and_wrong_replies_are_rejected_before_writes() {
    let mut s = state();
    let mut c = Invocation::new(1, Function::Flags { mask: 1, value: 1 });
    let r = c.advance(&mut s).unwrap().unwrap();
    let before = (c.clone(), s.clone());
    assert!(c.acknowledge(&r, Reply::Done).is_err());
    assert_eq!((c.clone(), s.clone()), before);
    let mut v = json!(c);
    v["pc"] = json!(99);
    let mut bad: Invocation = serde_json::from_value(v).unwrap();
    assert!(bad.advance(&mut s).is_err());
    assert_eq!(s, before.1);
    let mut c = Invocation::new(9, Function::SwitchFocus { target: 2 });
    let r = c.advance(&mut s).unwrap().unwrap();
    c.acknowledge(&r, Reply::Registered(false)).unwrap();
    let r = c.advance(&mut s).unwrap().unwrap();
    c.acknowledge(&r, Reply::Registered(true)).unwrap();
    c.advance(&mut s).unwrap();
    let mut v = json!(c);
    v["pending"]["Event"]["window"] = json!(3);
    let mut bad: Invocation = serde_json::from_value(v).unwrap();
    let before = s.clone();
    assert!(bad.advance(&mut s).is_err());
    assert_eq!(s, before);
}

#[test]
fn panel_selection_composes_base_visibility_flags_invalidation_and_focus() {
    use sims_vm::interface::panel_switch::{Fields as Panels, Request as PanelRequest, Selection};
    use sims_vm::ui_effect::widgets::visibility::Visibility;
    let mut s = state();
    for id in 1..=7 {
        s.windows.insert(
            id,
            Fields {
                parent: if id == 7 { 0 } else { 7 },
                dirty: 0,
                flags: 1,
            },
        );
    }
    s.managers.get_mut(&9).unwrap().focus = 5;
    s.managers.get_mut(&9).unwrap().root = 7;
    s.managers.get_mut(&9).unwrap().registered = (1..=7).collect();
    let mut fields = Panels {
        selected: 5,
        panels: [1, 2, 3, 4, 5, 6],
    };
    let mut selection = Selection::new(0);
    let mut events = vec![];
    while let Some(r) = selection.advance(&mut fields).unwrap() {
        (selection, fields, s) = saved((selection, fields, s));
        match r {
            PanelRequest::Visibility { panel, show } => {
                let mut child = Visibility::new(panel, show);
                while let Some(c) = child.advance(0).unwrap() {
                    (selection, child, fields, s) = saved((selection, child, fields, s));
                    let function = match c.slot {
                        0x94 => Function::Show,
                        0x98 => Function::Hide,
                        _ => panic!("unexpected policy branch"),
                    };
                    compose(Invocation::new(c.window, function), &mut s, &mut events, 0);
                    child.acknowledge(&c).unwrap();
                }
                assert!(child.complete());
            }
            PanelRequest::Invalidate(w) => window::invalidate(s.windows.get_mut(&w).unwrap()),
        }
        selection.acknowledge(r).unwrap();
    }
    assert!(selection.complete());
    assert_eq!(fields.selected, 0);
    assert_eq!(s.managers[&9].focus, 7);
    assert_eq!(
        events,
        vec![
            Request::Event {
                window: 5,
                words: [20, 7, 0, 0]
            },
            Request::Event {
                window: 7,
                words: [21, 5, 0, 0]
            }
        ]
    );
    for id in 1..=7 {
        assert_eq!(s.windows[&id].flags, u32::from(id == 1 || id == 7));
        assert_eq!(s.windows[&id].dirty, 1);
    }
}

#[test]
fn base_focus_events_forward_only_other_window_and_accept_without_state_writes() {
    for other in [0, 7, u32::MAX] {
        for (message, slot) in [(20, 0x134), (21, 0x130)] {
            assert_eq!(
                window::focus_event(1, [message, other, 99, 123]),
                Some(Call::new(1, slot, &[other]))
            );
            assert_eq!(window::accept_focus(other), 1);
        }
    }
    assert_eq!(window::focus_event(1, [26, 0, 0, 0]), None);
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
#[ignore = "private prepared inputs; Rust continuations, no original execution"]
fn prepared_window_visibility_cases_restore_each_child() {
    let v = corpus("native-window-visibility-inputs.json");
    assert_eq!(v["native_execution"], false);
    assert_eq!(v["cases"].as_array().unwrap().len(), 508);
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case{n}: {c}");
    }
}
#[test]
#[ignore = "guarded original window/focus caller corpus; explicit service boundaries"]
fn window_visibility_and_focus_match_original_callers() {
    let v = corpus("research/sims-exe/oracle/window-visibility.json");
    assert_eq!(v["cases"].as_array().unwrap().len(), 508);
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
