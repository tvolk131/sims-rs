use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::ui_effect::widgets::{
    layout::{Fields, Function, Invocation, Link, Reply, Request, State},
    visibility::Call,
};
use std::collections::BTreeMap;

fn saved<T: Serialize + DeserializeOwned>(value: T) -> T {
    serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap()
}
fn state() -> State {
    State {
        windows: (1..=4)
            .map(|id| {
                (
                    id,
                    Fields {
                        rectangle: [id as i32, 2, 11, 22],
                        absolute: [-1; 4],
                        sentinel: id + 100,
                        updating: 0,
                        surface: 0,
                        owned: 0,
                    },
                )
            })
            .collect(),
        links: (1..=4)
            .map(|id| {
                (
                    id + 100,
                    Link {
                        next: id + 100,
                        window: 0,
                    },
                )
            })
            .collect(),
        global_manager: 9,
        roots: BTreeMap::from([(9, 4), (10, 3)]),
    }
}
fn take(call: &mut Invocation, s: &mut State, expected: Request, reply: Reply) {
    assert_eq!(call.advance(s).unwrap(), Some(expected.clone()));
    *call = saved(call.clone());
    assert_eq!(call.advance(s).unwrap(), Some(expected.clone()));
    call.acknowledge(&expected, reply).unwrap();
    *call = saved(call.clone());
}
fn virtual_call(window: u32, slot: u16, args: &[u32]) -> Request {
    Request::Virtual(Call::new(window, slot, args))
}

#[test]
fn move_retains_wrapping_rectangle_and_does_not_write_before_virtual_setter() {
    let mut s = state();
    s.windows.get_mut(&1).unwrap().rectangle = [i32::MIN, 10, i32::MAX, -20];
    let before = s.clone();
    let mut call = Invocation::new(
        1,
        Function::Move {
            x: i32::MAX,
            y: i32::MIN,
        },
    );
    let expected = virtual_call(
        1,
        0x60,
        &[
            i32::MAX as u32,
            i32::MIN as u32,
            (i32::MAX - 1) as u32,
            i32::MIN.wrapping_sub(30) as u32,
        ],
    );
    assert_eq!(call.advance(&mut s).unwrap(), Some(expected.clone()));
    assert_eq!(s, before);
    s.windows.get_mut(&1).unwrap().rectangle = [0; 4];
    call = saved(call);
    assert_eq!(call.advance(&mut s).unwrap(), Some(expected.clone()));
    call.acknowledge(&expected, Reply::Done).unwrap();
    assert_eq!(call.advance(&mut s).unwrap(), None);
}

#[test]
fn rectangle_writes_first_retains_resize_and_clears_current_resource_after_disposal() {
    let mut s = state();
    s.windows.get_mut(&1).unwrap().surface = 70;
    s.windows.get_mut(&1).unwrap().owned = 80;
    let new = [10, 20, 40, 60];
    let mut call = Invocation::new(1, Function::Rectangle { rectangle: new });
    let first = virtual_call(1, 0x174, &[1, 0]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(first.clone()));
    assert_eq!(s.window(1).unwrap().rectangle, new);
    s.windows.get_mut(&1).unwrap().rectangle = [1, 2, 11, 22];
    call.acknowledge(&first, Reply::Done).unwrap();
    take(&mut call, &mut s, virtual_call(1, 0x198, &[]), Reply::Done);
    take(&mut call, &mut s, virtual_call(1, 0x88, &[]), Reply::Done);
    s.windows.get_mut(&1).unwrap().owned = 81;
    let dispose = Request::Dispose { resource: 81 };
    assert_eq!(call.advance(&mut s).unwrap(), Some(dispose.clone()));
    s.windows.get_mut(&1).unwrap().owned = 82;
    call = saved(call);
    call.acknowledge(&dispose, Reply::Done).unwrap();
    let flags = virtual_call(1, 0x8c, &[8]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(flags.clone()));
    assert_eq!(s.window(1).unwrap().owned, 0);
    s.global_manager = 10;
    s.windows.get_mut(&1).unwrap().absolute = [5, 6, 7, 8];
    call.acknowledge(&flags, Reply::Word(257)).unwrap();
    take(
        &mut call,
        &mut s,
        Request::RootRegion { root: 3, window: 1 },
        Reply::Done,
    );
    assert_eq!(call.advance(&mut s).unwrap(), None);
}

#[test]
fn same_size_rectangle_still_refreshes_but_preserves_resource_and_checks_low_byte() {
    let mut s = state();
    s.windows.get_mut(&1).unwrap().owned = 80;
    s.windows.get_mut(&1).unwrap().surface = 70;
    let mut call = Invocation::new(
        1,
        Function::Rectangle {
            rectangle: [11, 12, 21, 32],
        },
    );
    take(&mut call, &mut s, virtual_call(1, 0x198, &[]), Reply::Done);
    take(&mut call, &mut s, virtual_call(1, 0x88, &[]), Reply::Done);
    take(
        &mut call,
        &mut s,
        virtual_call(1, 0x8c, &[8]),
        Reply::Word(256),
    );
    assert_eq!(call.advance(&mut s).unwrap(), None);
    assert_eq!(s.window(1).unwrap().owned, 80);
}

#[test]
fn recursive_flags_capture_node_but_reread_successor_and_replaced_sentinel() {
    let mut s = state();
    s.links.get_mut(&101).unwrap().next = 201;
    s.links.insert(
        201,
        Link {
            next: 202,
            window: 2,
        },
    );
    s.links.insert(
        202,
        Link {
            next: 101,
            window: 3,
        },
    );
    let mut call = Invocation::new(1, Function::Flag4000 { value: 257 });
    take(
        &mut call,
        &mut s,
        virtual_call(1, 0x90, &[0x4000, 257]),
        Reply::Word(0x4000),
    );
    let child = Request::Flag4000 {
        window: 2,
        value: 257,
    };
    assert_eq!(call.advance(&mut s).unwrap(), Some(child.clone()));
    s.links.get_mut(&201).unwrap().next = 303;
    s.links.insert(
        303,
        Link {
            next: 104,
            window: 4,
        },
    );
    s.windows.get_mut(&1).unwrap().sentinel = 104;
    call = saved(call);
    call.acknowledge(&child, Reply::Done).unwrap();
    take(
        &mut call,
        &mut s,
        Request::Flag4000 {
            window: 4,
            value: 257,
        },
        Reply::Done,
    );
    assert_eq!(call.advance(&mut s).unwrap(), None);
}

#[test]
fn absolute_layout_captures_self_before_parent_callbacks_but_reads_each_parent_fresh() {
    let mut s = state();
    s.windows.get_mut(&1).unwrap().updating = 128;
    s.links.get_mut(&101).unwrap().next = 201;
    s.links.insert(
        201,
        Link {
            next: 101,
            window: 4,
        },
    );
    let mut call = Invocation::new(1, Function::Absolute);
    take(
        &mut call,
        &mut s,
        virtual_call(1, 0x18, &[]),
        Reply::Word(2),
    );
    s.windows.get_mut(&1).unwrap().rectangle = [90; 4];
    s.windows.get_mut(&2).unwrap().rectangle = [10, 20, 30, 40];
    take(
        &mut call,
        &mut s,
        virtual_call(2, 0x18, &[]),
        Reply::Word(3),
    );
    s.windows.get_mut(&2).unwrap().rectangle = [99; 4];
    s.windows.get_mut(&3).unwrap().rectangle = [-5, -7, 0, 0];
    take(
        &mut call,
        &mut s,
        virtual_call(3, 0x18, &[]),
        Reply::Word(0),
    );
    let child = virtual_call(4, 0x88, &[]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(child.clone()));
    assert_eq!(s.window(1).unwrap().absolute, [6, 15, 16, 35]);
    assert_eq!(s.window(1).unwrap().updating, 128);
    call = saved(call);
    call.acknowledge(&child, Reply::Done).unwrap();
    assert_eq!(call.advance(&mut s).unwrap(), None);
    assert_eq!(s.window(1).unwrap().updating, 0);
}

#[test]
fn refresh_reads_children_after_local_callback_and_rejects_wrong_reply_without_writes() {
    let mut s = state();
    let mut call = Invocation::new(1, Function::Refresh);
    let first = virtual_call(1, 0x194, &[]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(first.clone()));
    let before = (call.clone(), s.clone());
    assert!(call.acknowledge(&first, Reply::Word(1)).is_err());
    assert_eq!((call.clone(), s.clone()), before);
    s.links.get_mut(&101).unwrap().next = 201;
    s.links.insert(
        201,
        Link {
            next: 101,
            window: 2,
        },
    );
    call.acknowledge(&first, Reply::Done).unwrap();
    take(&mut call, &mut s, virtual_call(2, 0x198, &[]), Reply::Done);
    assert_eq!(call.advance(&mut s).unwrap(), None);
    let mut malformed = json!(Invocation::new(1, Function::Refresh));
    malformed["stage"] = json!("Owned");
    let mut bad: Invocation = serde_json::from_value(malformed).unwrap();
    let before = s.clone();
    assert!(bad.advance(&mut s).is_err());
    assert_eq!(s, before);
}

#[test]
fn move_composes_rectangle_refresh_and_absolute_children_with_saved_call_stack() {
    let mut s = state();
    s.links.get_mut(&101).unwrap().next = 201;
    s.links.insert(
        201,
        Link {
            next: 202,
            window: 2,
        },
    );
    s.links.insert(
        202,
        Link {
            next: 101,
            window: 3,
        },
    );
    let parents = BTreeMap::from([(1, 4), (2, 1), (3, 1), (4, 0)]);
    let mut stack = vec![Invocation::new(1, Function::Move { x: 10, y: 20 })];
    let mut local_surface_requests = vec![];
    while !stack.is_empty() {
        (stack, s) = saved((stack, s));
        let Some(r) = stack.last_mut().unwrap().advance(&mut s).unwrap() else {
            stack.pop();
            if let Some(parent) = stack.last_mut() {
                let r = parent.request().unwrap().clone();
                parent.acknowledge(&r, Reply::Done).unwrap();
            }
            continue;
        };
        let Request::Virtual(c) = &r else {
            panic!("unexpected service {r:?}")
        };
        let child = match c.slot {
            0x60 => Some(Function::Rectangle {
                rectangle: c
                    .arguments
                    .iter()
                    .map(|x| *x as i32)
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap(),
            }),
            0x198 => Some(Function::Refresh),
            0x88 => Some(Function::Absolute),
            _ => None,
        };
        if let Some(function) = child {
            stack.push(Invocation::new(c.window, function));
            continue;
        }
        let reply = match c.slot {
            0x18 => Reply::Word(parents[&c.window]),
            0x8c => Reply::Word(0),
            // Explicit service boundary: this test covers traversal/caller
            // composition, not the as-yet-unimplemented virtual194 body.
            0x194 => {
                local_surface_requests.push(c.window);
                Reply::Done
            }
            _ => panic!("unexpected virtual {c:?}"),
        };
        stack.last_mut().unwrap().acknowledge(&r, reply).unwrap();
    }
    assert_eq!(local_surface_requests, [1, 2, 3]);
    assert_eq!(s.window(1).unwrap().rectangle, [10, 20, 20, 40]);
    assert_eq!(s.window(1).unwrap().absolute, [14, 22, 24, 42]);
    assert_eq!(s.window(2).unwrap().absolute, [16, 24, 25, 44]);
    assert_eq!(s.window(3).unwrap().absolute, [17, 24, 25, 44]);
    assert!(s.windows.values().all(|f| f.updating == 0));
}

#[test]
fn recursive_flag_walk_composes_concrete_flag_setter_across_saved_nested_frames() {
    use sims_vm::ui_effect::widgets::window;
    let mut s = state();
    s.links.get_mut(&101).unwrap().next = 201;
    s.links.insert(
        201,
        Link {
            next: 101,
            window: 2,
        },
    );
    s.links.get_mut(&102).unwrap().next = 202;
    s.links.insert(
        202,
        Link {
            next: 102,
            window: 3,
        },
    );
    let mut base = window::State {
        global_manager: 9,
        windows: (1..=4)
            .map(|id| {
                (
                    id,
                    window::Fields {
                        parent: 0,
                        dirty: 128,
                        flags: 1,
                    },
                )
            })
            .collect(),
        managers: BTreeMap::new(),
    };
    let mut stack = vec![Invocation::new(1, Function::Flag4000 { value: 257 })];
    let mut visits = vec![];
    while !stack.is_empty() {
        (stack, s, base) = saved((stack, s, base));
        let Some(r) = stack.last_mut().unwrap().advance(&mut s).unwrap() else {
            stack.pop();
            if let Some(parent) = stack.last_mut() {
                let r = parent.request().unwrap().clone();
                parent.acknowledge(&r, Reply::Done).unwrap();
            }
            continue;
        };
        match &r {
            Request::Flag4000 { window, value } => {
                stack.push(Invocation::new(
                    *window,
                    Function::Flag4000 { value: *value },
                ));
            }
            Request::Virtual(c) => {
                assert_eq!(c.slot, 0x90);
                visits.push(c.window);
                let mut flags = window::Invocation::new(
                    c.window,
                    window::Function::Flags {
                        mask: c.arguments[0],
                        value: c.arguments[1],
                    },
                );
                assert_eq!(flags.advance(&mut base).unwrap(), None);
                stack
                    .last_mut()
                    .unwrap()
                    .acknowledge(&r, Reply::Word(flags.result().unwrap()))
                    .unwrap();
            }
            _ => panic!("unexpected service"),
        }
    }
    assert_eq!(visits, [1, 2, 3]);
    for id in [1, 2, 3] {
        assert_eq!(base.window(id).unwrap().flags, 0x4001);
        assert_eq!(base.window(id).unwrap().dirty, 128);
    }
    assert_eq!(base.window(4).unwrap().flags, 1);
}

fn merge(to: &mut Value, patch: &Value) {
    if let Some(o) = patch.as_object() {
        for (k, v) in o {
            if v.is_object() {
                merge(&mut to[k], v)
            } else {
                to[k] = v.clone();
            }
        }
    }
}
fn supplied(c: &Value, s: &State, r: &Request) -> Reply {
    match r {
        Request::Virtual(v) if v.slot == 0x18 => {
            Reply::Word(c["parents"][v.window.to_string()].as_u64().unwrap_or(0) as u32)
        }
        Request::Virtual(v) if v.slot == 0x8c => {
            Reply::Word(c["flag_reply"].as_u64().unwrap_or(0) as u32)
        }
        Request::Virtual(v) if v.slot == 0x90 => Reply::Word(s.window(v.window).unwrap().surface), //ignored return, arbitrary original service word
        _ => Reply::Done,
    }
}
fn run(c: &Value, restore: bool) -> Value {
    let mut s: State = serde_json::from_value(c["state"].clone()).unwrap();
    let mut stack = vec![Invocation::new(
        c["owner"].as_u64().unwrap() as u32,
        serde_json::from_value(c["function"].clone()).unwrap(),
    )];
    let mut events = vec![];
    while !stack.is_empty() {
        if restore {
            (stack, s) = saved((stack, s));
        }
        let call = stack.last_mut().unwrap();
        let Some(r) = call.advance(&mut s).unwrap() else {
            stack.pop();
            if let Some(parent) = stack.last_mut() {
                let r = parent.request().unwrap().clone();
                parent.acknowledge(&r, Reply::Done).unwrap();
            }
            continue;
        };
        if let Request::Flag4000 { window, value } = r {
            stack.push(Invocation::new(window, Function::Flag4000 { value }));
            continue;
        }
        let n = events.len().to_string();
        let reply = c["replies"]
            .get(&n)
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap())
            .unwrap_or_else(|| supplied(c, &s, &r));
        let mut x = json!(s);
        events.push(json!({"request":r,"state":x}));
        merge(&mut x, &c["patches"][&n]);
        s = serde_json::from_value(x).unwrap();
        if restore {
            (stack, s) = saved((stack, s));
        }
        stack.last_mut().unwrap().acknowledge(&r, reply).unwrap();
        assert!(events.len() < 1000);
    }
    json!({"events":events,"state":s})
}
fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "private Rust continuation matrix; does not execute the original"]
fn prepared_window_layout_cases_restore_every_child() {
    let file = root().join("cache/native-window-layout-inputs.json");
    let all: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 858);
    for (n, c) in cases.iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case{n}");
    }
}

#[test]
#[ignore = "guarded native layout caller and recursive flag comparison"]
fn window_layout_matches_original_callers() {
    let all: Value = serde_json::from_slice(
        &std::fs::read(root().join("cache/research/sims-exe/oracle/window-layout.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        all["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        all["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(all["cases"].as_array().unwrap().len(), 858);
    for (n, c) in all["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(&c["input"], true), c["original"], "case{n}");
    }
}
