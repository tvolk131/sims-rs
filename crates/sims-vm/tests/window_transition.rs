use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::ui_effect::widgets::{
    layout, surface_geometry as geom, surface_resource as res,
    transition::{Fields, Function, Invocation, Reply, Request, State, curve},
    visibility::Call,
};
use std::collections::BTreeMap;

fn saved<T: Serialize + DeserializeOwned>(v: T) -> T {
    serde_json::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap()
}
fn state() -> State {
    State {
        resources: res::State {
            geometry: geom::State {
                layout: layout::State {
                    windows: (1..=4)
                        .map(|id| {
                            (
                                id,
                                layout::Fields {
                                    rectangle: [1, 2, 11, 22],
                                    absolute: [0; 4],
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
                                layout::Link {
                                    next: id + 100,
                                    window: 0,
                                },
                            )
                        })
                        .collect(),
                    global_manager: 9,
                    roots: BTreeMap::from([(9, 4)]),
                },
                geometry: (1..=4)
                    .map(|id| {
                        (
                            id,
                            geom::Fields {
                                local: [0; 4],
                                relative: [0; 4],
                            },
                        )
                    })
                    .collect(),
                regions: BTreeMap::new(),
            },
            drawing: (1..=4).map(|id| (id, 0)).collect(),
            backend: 20,
            resources: BTreeMap::new(),
            formats: BTreeMap::new(),
        },
        transitions: (1..=4)
            .map(|id| {
                (
                    id,
                    Fields {
                        direction: 0,
                        curve: [0; 4],
                        had_surface: 0,
                        option: 0,
                    },
                )
            })
            .collect(),
        now: 100,
    }
}
fn wc(window: u32, slot: u16, arguments: &[u32]) -> Request {
    Request::Window(Call::new(window, slot, arguments))
}
fn take(call: &mut Invocation, s: &mut State, r: Request, reply: Reply) {
    assert_eq!(call.advance(s).unwrap(), Some(r.clone()));
    (*s, *call) = saved((s.clone(), call.clone()));
    assert_eq!(call.advance(s).unwrap(), Some(r.clone()));
    let wrong = if matches!(reply, Reply::Word(_)) {
        Reply::Done
    } else {
        Reply::Word(0)
    };
    let before = call.clone();
    assert!(call.acknowledge(&r, wrong).is_err());
    assert_eq!(*call, before);
    call.acknowledge(&r, reply).unwrap();
    *call = saved(call.clone());
}
fn visibility(show: u32, duration: i32) -> Invocation {
    Invocation::new(
        1,
        Function::Visibility {
            show,
            option: 0x102,
            duration,
        },
    )
}
fn end(call: &mut Invocation, s: &mut State) {
    assert_eq!(call.advance(s).unwrap(), None);
    assert!(call.complete());
}
#[test]
fn fresh_transition_captures_surface_before_callback_and_uses_current_clock() {
    let mut s = state();
    let mut call = visibility(257, 0);
    take(&mut call, &mut s, wc(1, 0x8c, &[1]), Reply::Word(256));
    let r = wc(1, 0x174, &[1, 0]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
    assert_eq!(
        (s.transitions[&1].option, s.transitions[&1].had_surface),
        (2, 0)
    );
    s.now = 500;
    s.resources
        .geometry
        .layout
        .windows
        .get_mut(&1)
        .unwrap()
        .surface = 11;
    call.acknowledge(&r, Reply::Done).unwrap();
    take(&mut call, &mut s, wc(1, 0x94, &[]), Reply::Done);
    assert_eq!(s.transitions[&1].curve, curve::configure(0, true, 333, 500));
    assert_eq!(s.transitions[&1].direction, 1);
    // Show's writes are not replaced on return.
    s.transitions.get_mut(&1).unwrap().direction = 7;
    take(&mut call, &mut s, wc(1, 0x18, &[]), Reply::Word(2));
    take(&mut call, &mut s, wc(1, 0x18, &[]), Reply::Word(3));
    take(&mut call, &mut s, wc(3, 0x168, &[]), Reply::Done);
    take(&mut call, &mut s, wc(1, 0x168, &[]), Reply::Done);
    end(&mut call, &mut s);
    assert_eq!(s.transitions[&1].direction, 7);
}
#[test]
fn visibility_compares_literal_low_bytes_and_avoids_unnecessary_reversals() {
    for show in [0, 1, 2, 256, 257, 258] {
        for visible in [0, 1, 2, 256, 257, 258] {
            for direction in [-1, 0, 1] {
                let mut s = state();
                s.transitions.get_mut(&1).unwrap().direction = direction;
                s.transitions.get_mut(&1).unwrap().curve = curve::configure(0, true, 6, 98);
                let mut call = visibility(show, 1);
                take(&mut call, &mut s, wc(1, 0x8c, &[1]), Reply::Word(visible));
                let b = show as u8;
                let v = visible as u8;
                let skip = (b == v && (b == 0 || direction >= 0))
                    || (b == 0 && direction < 0)
                    || (b != 0 && direction > 0);
                let before = s.clone();
                let next = call.advance(&mut s).unwrap();
                if skip {
                    assert!(call.complete());
                    assert_eq!(s, before);
                } else if direction != 0 {
                    assert!(call.complete());
                    assert_eq!(s.transitions[&1].direction, -direction);
                } else {
                    assert_eq!(next, Some(wc(1, 0x174, &[1, 0])));
                }
            }
        }
    }
}
#[test]
fn reversal_duration_uses_unspilled_sample_and_keeps_exceptional_slopes() {
    let c = curve::configure(0, true, 6, 0);
    assert_eq!(curve::sample_bits(c, 3).unwrap(), 0x3f000000);
    // The stored 1/6 slope yields a sample just below 0.5 before f32 spill.
    // Adding 0.5 then truncating must produce zero, not one.
    assert_eq!(
        curve::reverse(c, 1, 1, 3).unwrap(),
        [0, 0x7f800000, 0xff800000, 3]
    );
    assert_eq!(
        curve::configure(0, false, 0, 3),
        [0, 0x7fc00000, 0xffc00000, 3]
    );
    assert_eq!(
        curve::sample_bits([0, 0, 0xff800000, 4], 3).unwrap(),
        0x7f800000
    );
    assert_eq!(curve::sample_bits([0, 0, 0xff800000, 4], 4).unwrap(), 0);
    assert_eq!(
        curve::configure(0, false, -1, 0),
        [0, 0x80000000, 0x80000000, u32::MAX]
    );
    let wrapped = curve::configure(0, true, 10, u32::MAX - 2);
    assert_eq!(wrapped[3], 7);
    assert_eq!(
        curve::sample_bits(wrapped, u32::MAX - 1).unwrap(),
        0x3f800000
    );
    assert!(curve::sample_bits([0x40000000, 0, 0, 1], 0).is_err());
}
#[test]
fn expiration_and_explicit_cancellation_keep_distinct_cleanup_order() {
    for cancel in [false, true] {
        let mut s = state();
        s.transitions.get_mut(&1).unwrap().direction = -1;
        s.resources
            .geometry
            .layout
            .windows
            .get_mut(&1)
            .unwrap()
            .owned = 55;
        let mut call = if cancel {
            Invocation::new(1, Function::Cancel)
        } else {
            visibility(0, 1)
        };
        take(&mut call, &mut s, wc(1, 0x98, &[]), Reply::Done);
        if cancel {
            take(&mut call, &mut s, wc(1, 0x174, &[0, 0]), Reply::Done);
        }
        take(&mut call, &mut s, wc(1, 0x18, &[]), Reply::Word(2));
        let r = wc(1, 0x18, &[]);
        assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
        s.transitions.get_mut(&1).unwrap().direction = 12;
        s.resources.geometry.layout.global_manager = 10;
        call.acknowledge(&r, Reply::Word(3)).unwrap();
        take(
            &mut call,
            &mut s,
            Request::Notify {
                manager: 10,
                record: [3, 27, 1, u32::MAX, 0],
            },
            Reply::Done,
        );
        let r = Request::Dispose { resource: 55 };
        assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
        assert_eq!(s.transitions[&1].direction, 0);
        assert_eq!(s.resources.geometry.layout.windows[&1].owned, 55);
        s.resources
            .geometry
            .layout
            .windows
            .get_mut(&1)
            .unwrap()
            .owned = 56;
        call.acknowledge(&r, Reply::Done).unwrap();
        if !cancel {
            take(&mut call, &mut s, wc(1, 0x174, &[0, 0]), Reply::Done);
            take(&mut call, &mut s, wc(1, 0x8c, &[1]), Reply::Word(0));
        }
        end(&mut call, &mut s);
        assert_eq!(s.resources.geometry.layout.windows[&1].owned, 0);
    }
}
#[test]
fn cancellation_walks_live_children_even_when_inactive_and_keeps_pending_work() {
    let mut s = state();
    let l = &mut s.resources.geometry.layout;
    l.links.get_mut(&101).unwrap().next = 110;
    l.links.insert(
        110,
        layout::Link {
            next: 111,
            window: 2,
        },
    );
    l.links.insert(
        111,
        layout::Link {
            next: 101,
            window: 3,
        },
    );
    l.links.insert(
        112,
        layout::Link {
            next: 104,
            window: 4,
        },
    );
    let mut call = Invocation::new(1, Function::Cancel);
    take(&mut call, &mut s, wc(2, 0xac, &[]), Reply::Done);
    s.resources
        .geometry
        .layout
        .links
        .get_mut(&110)
        .unwrap()
        .next = 112;
    s.resources
        .geometry
        .layout
        .windows
        .get_mut(&1)
        .unwrap()
        .sentinel = 104;
    take(&mut call, &mut s, wc(4, 0xac, &[]), Reply::Done);
    end(&mut call, &mut s);
}
#[test]
fn second_parent_null_skips_notification_but_is_not_skipped_for_invalidation() {
    let mut s = state();
    s.transitions.get_mut(&1).unwrap().direction = 1;
    s.transitions.get_mut(&1).unwrap().had_surface = 1;
    let mut call = Invocation::new(1, Function::Cancel);
    take(&mut call, &mut s, wc(1, 0x18, &[]), Reply::Word(2));
    take(&mut call, &mut s, wc(1, 0x18, &[]), Reply::Word(0));
    end(&mut call, &mut s);
    s.resources
        .geometry
        .layout
        .windows
        .get_mut(&1)
        .unwrap()
        .surface = 11;
    let mut call = visibility(0, 1);
    take(&mut call, &mut s, wc(1, 0x8c, &[1]), Reply::Word(1));
    take(&mut call, &mut s, wc(1, 0x18, &[]), Reply::Word(2));
    take(&mut call, &mut s, wc(1, 0x18, &[]), Reply::Word(0));
    // This request must fail in a real owner resolver; no synthetic null success.
    assert_eq!(call.advance(&mut s).unwrap(), Some(wc(0, 0x168, &[])));
}

#[test]
fn transitions_compose_real_show_hide_flags_and_queued_notifications() {
    use serde::Deserialize;
    use sims_vm::ui_effect::widgets::window as base;
    #[derive(Clone, Serialize, Deserialize)]
    enum Frame {
        Transition(Invocation),
        Base(base::Invocation),
    }
    let mut s = state();
    s.resources
        .geometry
        .layout
        .windows
        .get_mut(&1)
        .unwrap()
        .surface = 11;
    let mut b = base::State {
        global_manager: 9,
        windows: (1..=4)
            .map(|id| {
                (
                    id,
                    base::Fields {
                        parent: if id == 1 { 2 } else { 0 },
                        dirty: 0,
                        flags: 0,
                    },
                )
            })
            .collect(),
        managers: BTreeMap::from([(
            9,
            base::Manager {
                focus: 0,
                root: 2,
                special: 0,
                registered: vec![1, 2],
            },
        )]),
    };
    let mut messages = vec![];
    let mut run = |function, s: &mut State, b: &mut base::State| {
        let mut stack = vec![Frame::Transition(Invocation::new(1, function))];
        while !stack.is_empty() {
            let mut child = None;
            match stack.last_mut().unwrap() {
                Frame::Transition(c) => {
                    if let Some(r) = c.advance(s).unwrap() {
                        let reply = match &r {
                            Request::Window(w) => match w.slot {
                                0x94 | 0x98 => {
                                    child = Some(Frame::Base(base::Invocation::new(
                                        w.window,
                                        if w.slot == 0x94 {
                                            base::Function::Show
                                        } else {
                                            base::Function::Hide
                                        },
                                    )));
                                    Reply::Done
                                }
                                0x18 => Reply::Word(base::parent(b.window(w.window).unwrap())),
                                0x8c => Reply::Word(base::flags(
                                    b.window(w.window).unwrap(),
                                    w.arguments[0],
                                )),
                                0x168 => {
                                    base::invalidate(b.windows.get_mut(&w.window).unwrap());
                                    Reply::Done
                                }
                                _ => panic!("unexpected transition child {r:?}"),
                            },
                            Request::Notify { manager, record } => {
                                messages.push((*manager, *record));
                                Reply::Done
                            }
                            _ => panic!("unexpected transition request {r:?}"),
                        };
                        if child.is_none() {
                            c.acknowledge(&r, reply).unwrap();
                        }
                    } else {
                        assert!(c.complete());
                        stack.pop();
                    }
                }
                Frame::Base(c) => {
                    if let Some(r) = c.advance(b).unwrap() {
                        let reply = match &r {
                            base::Request::Virtual(w) => match w.slot {
                                0x90 => {
                                    child = Some(Frame::Base(base::Invocation::new(
                                        w.window,
                                        base::Function::Flags {
                                            mask: w.arguments[0],
                                            value: w.arguments[1],
                                        },
                                    )));
                                    base::Reply::Done
                                }
                                0x18 => {
                                    base::Reply::Word(base::parent(b.window(w.window).unwrap()))
                                }
                                0x8c => base::Reply::Word(base::flags(
                                    b.window(w.window).unwrap(),
                                    w.arguments[0],
                                )),
                                0x168 => {
                                    base::invalidate(b.windows.get_mut(&w.window).unwrap());
                                    base::Reply::Done
                                }
                                _ => panic!("unexpected base child {r:?}"),
                            },
                            _ => panic!("unexpected base request {r:?}"),
                        };
                        if child.is_none() {
                            c.acknowledge(&r, reply).unwrap();
                        }
                    } else {
                        assert!(c.complete());
                        let result = c.result();
                        stack.pop();
                        match stack.last_mut().unwrap() {
                            Frame::Transition(parent) => {
                                let r = parent.request().unwrap().clone();
                                parent.acknowledge(&r, Reply::Done).unwrap();
                            }
                            Frame::Base(parent) => {
                                let r = parent.request().unwrap().clone();
                                parent
                                    .acknowledge(&r, base::Reply::Word(result.unwrap()))
                                    .unwrap();
                            }
                        }
                    }
                }
            }
            if let Some(child) = child {
                stack.push(child);
            }
            // Includes the suspended transition parent during nested base work.
            (*s, *b, stack) = saved((s.clone(), b.clone(), stack));
        }
    };
    run(
        Function::Visibility {
            show: 1,
            option: 0,
            duration: 10,
        },
        &mut s,
        &mut b,
    );
    assert_eq!((s.transitions[&1].direction, b.windows[&1].flags), (1, 1));
    s.now = 110;
    run(
        Function::Visibility {
            show: 1,
            option: 0,
            duration: 10,
        },
        &mut s,
        &mut b,
    );
    assert_eq!((s.transitions[&1].direction, b.windows[&1].flags), (0, 1));
    run(
        Function::Visibility {
            show: 0,
            option: 0,
            duration: 10,
        },
        &mut s,
        &mut b,
    );
    assert_eq!((s.transitions[&1].direction, b.windows[&1].flags), (-1, 1));
    run(Function::Cancel, &mut s, &mut b);
    assert_eq!((s.transitions[&1].direction, b.windows[&1].flags), (0, 0));
    assert_eq!(
        messages,
        vec![(9, [2, 27, 1, 1, 0]), (9, [2, 27, 1, u32::MAX, 0])]
    );
    assert_eq!((b.windows[&1].dirty, b.windows[&2].dirty), (1, 1));
}
fn merge(v: &mut Value, p: &Value) {
    for (k, x) in p.as_object().unwrap() {
        if x.is_object() {
            merge(&mut v[k], x)
        } else {
            v[k] = x.clone();
        }
    }
}
fn supplied(c: &Value, s: &State, r: &Request) -> Reply {
    if let Request::Window(w) = r {
        if w.slot == 0x18 {
            return Reply::Word(c["parents"][w.window.to_string()].as_u64().unwrap_or(0) as u32);
        }
        if w.slot == 0x8c {
            return Reply::Word(c["flag_reply"].as_u64().unwrap() as u32);
        }
        assert!(s.resources.geometry.layout.windows.contains_key(&w.window));
    }
    Reply::Done
}
fn run(c: &Value, restore: bool) -> Value {
    let mut s: State = serde_json::from_value(c["state"].clone()).unwrap();
    let mut stack = vec![Invocation::new(
        c["owner"].as_u64().unwrap() as u32,
        serde_json::from_value(c["function"].clone()).unwrap(),
    )];
    let mut events = vec![];
    while !stack.is_empty() {
        assert!(events.len() < 200);
        let r = stack.last_mut().unwrap().advance(&mut s).unwrap();
        let Some(r) = r else {
            assert!(stack.pop().unwrap().complete());
            if let Some(parent) = stack.last_mut() {
                let r = parent.request().unwrap().clone();
                parent.acknowledge(&r, Reply::Done).unwrap();
            }
            continue;
        };
        if restore {
            (s, stack) = saved((s, stack));
            assert_eq!(stack.last().unwrap().request(), Some(&r));
        }
        if c["recursive"].as_bool().unwrap_or(false)
            && matches!(&r,Request::Window(w) if w.slot==0xac)
        {
            let Request::Window(w) = r else {
                unreachable!()
            };
            stack.push(Invocation::new(w.window, Function::Cancel));
            continue;
        }
        let n = events.len().to_string();
        events.push(json!({"request":r,"state":s}));
        let reply = if c["replies"][&n].is_null() {
            supplied(c, &s, &r)
        } else {
            serde_json::from_value(c["replies"][&n].clone()).unwrap()
        };
        if !c["patches"][&n].is_null() {
            let mut v = serde_json::to_value(&s).unwrap();
            merge(&mut v, &c["patches"][&n]);
            s = serde_json::from_value(v).unwrap();
        }
        stack.last_mut().unwrap().acknowledge(&r, reply).unwrap();
        if restore {
            (s, stack) = saved((s, stack));
        }
    }
    json!({"events":events,"state":s})
}
#[test]
#[ignore = "private prepared input matrix; Rust continuations, no original execution"]
fn prepared_transition_cases_restore_every_callback_without_duplicate_work() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../cache/native-window-transition-inputs.json"
    );
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], false);
    let rows = report["cases"].as_array().unwrap();
    assert!(rows.len() > 1000);
    for (n, c) in rows.iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case {n}");
    }
}
#[test]
#[ignore = "requires separately executed original instructions; prepared inputs are not native evidence"]
fn compare_original_transition_instructions() {
    let path = std::env::var("SIMS_NATIVE_WINDOW_TRANSITION").unwrap();
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            run(&row["input"], true),
            row["original"],
            "native transition {n}"
        );
    }
}
