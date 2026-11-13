use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::ui_effect::widgets::{
    layout, surface_geometry as geom,
    surface_resource::{self as res, Function, Invocation, Reply, Request, Resource, State},
    visibility::Call,
};
use std::collections::BTreeMap;
fn saved<T: Serialize + DeserializeOwned>(v: T) -> T {
    serde_json::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap()
}
fn state() -> State {
    State {
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
                                surface: if id == 1 { 11 } else { 0 },
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
        resources: (11..=13)
            .map(|id| {
                (
                    id,
                    Resource {
                        table: 1000 + id,
                        size: [10, 20],
                    },
                )
            })
            .collect(),
        formats: BTreeMap::from([(30, [16, 565]), (31, [32, 888])]),
    }
}
fn wc(window: u32, slot: u16, args: &[u32]) -> Request {
    Request::Window(Call::new(window, slot, args))
}
fn sc(owner: u32, table: u32, slot: u16, args: &[u32]) -> Request {
    Request::Surface {
        owner,
        table,
        slot,
        arguments: args.to_vec(),
    }
}
fn take(call: &mut Invocation, s: &mut State, r: Request, reply: Reply) {
    assert_eq!(call.advance(s).unwrap(), Some(r.clone()));
    *call = saved(call.clone());
    assert_eq!(call.advance(s).unwrap(), Some(r.clone()));
    call.acknowledge(&r, reply).unwrap();
    *call = saved(call.clone());
}
fn finish(call: &mut Invocation, s: &mut State, result: u8) {
    take(call, s, wc(1, 0x190, &[]), Reply::Done);
    take(call, s, wc(1, 0x168, &[]), Reply::Done);
    assert_eq!(call.advance(s).unwrap(), None);
    assert_eq!(call.result(), Some(result));
}
#[test]
fn same_size_ready_surface_skips_notifications_but_disabled_null_surface_still_notifies() {
    let mut s = state();
    let mut call = Invocation::new(
        1,
        Function::Ownership {
            enabled: 257,
            option: 0,
        },
    );
    take(&mut call, &mut s, sc(11, 1011, 0x2c, &[]), Reply::Word(257));
    assert_eq!(call.advance(&mut s).unwrap(), None);
    assert_eq!(call.result(), Some(1));
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 0;
    let mut call = Invocation::new(
        1,
        Function::Ownership {
            enabled: 256,
            option: 0,
        },
    );
    finish(&mut call, &mut s, 1);
}
#[test]
fn zero_dimensions_keep_uninitialized_resource_and_publish_failure() {
    for rectangle in [[1, 2, 1, 22], [1, 2, 11, 2]] {
        let mut s = state();
        s.geometry.layout.windows.get_mut(&1).unwrap().rectangle = rectangle;
        let mut call = Invocation::new(
            1,
            Function::Ownership {
                enabled: 1,
                option: 0,
            },
        );
        take(&mut call, &mut s, sc(11, 1011, 0x2c, &[]), Reply::Word(256));
        finish(&mut call, &mut s, 0);
        assert_eq!(s.geometry.layout.window(1).unwrap().surface, 11);
    }
}
#[test]
fn factory_result_overwrites_callback_replacement_and_null_result_still_notifies() {
    let mut s = state();
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 0;
    let mut call = Invocation::new(
        1,
        Function::Ownership {
            enabled: 1,
            option: 0,
        },
    );
    take(
        &mut call,
        &mut s,
        Request::BackendFactory { backend: 20 },
        Reply::Word(21),
    );
    let r = Request::FactoryCreate { factory: 21 };
    assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 12;
    call.acknowledge(&r, Reply::Word(0)).unwrap();
    let next = wc(1, 0x190, &[]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(next.clone()));
    assert_eq!(s.geometry.layout.window(1).unwrap().surface, 0);
    call.acknowledge(&next, Reply::Done).unwrap();
    take(&mut call, &mut s, wc(1, 0x168, &[]), Reply::Done);
    assert_eq!(call.advance(&mut s).unwrap(), None);
    assert_eq!(call.result(), Some(0));
}
#[test]
fn allocation_and_palette_keep_captured_method_table_but_reread_resource_receiver() {
    let mut s = state();
    let mut call = Invocation::new(
        1,
        Function::Ownership {
            enabled: 1,
            option: 257,
        },
    );
    take(&mut call, &mut s, sc(11, 1011, 0x2c, &[]), Reply::Word(0));
    take(
        &mut call,
        &mut s,
        Request::BackendFactory { backend: 20 },
        Reply::Word(21),
    );
    let format = Request::FactoryFormat { factory: 21 };
    assert_eq!(call.advance(&mut s).unwrap(), Some(format.clone()));
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 12;
    s.geometry.layout.windows.get_mut(&1).unwrap().rectangle = [0, 0, 999, 999];
    s.resources.get_mut(&11).unwrap().table = 999;
    s.formats.insert(31, [24, 123]);
    call.acknowledge(&format, Reply::Word(31)).unwrap();
    take(
        &mut call,
        &mut s,
        sc(12, 1011, 0xc, &[10, 20, 24, 123, 0, 2, 0]),
        Reply::Word(257),
    );
    take(&mut call, &mut s, wc(1, 0x8c, &[256]), Reply::Word(257));
    let palette = Request::Palette {
        owner: res::palette_owner(),
        index: 0,
    };
    assert_eq!(call.advance(&mut s).unwrap(), Some(palette.clone()));
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 13;
    s.resources.get_mut(&12).unwrap().table = 888;
    call.acknowledge(&palette, Reply::Word(0xabcdef01)).unwrap();
    take(
        &mut call,
        &mut s,
        sc(13, 1012, 0x68, &[0xabcdef01]),
        Reply::Done,
    );
    finish(&mut call, &mut s, 1);
}
#[test]
fn disable_uses_fresh_resource_at_each_callback_and_clears_replacement_after_disposal() {
    let mut s = state();
    let mut call = Invocation::new(
        1,
        Function::Ownership {
            enabled: 0,
            option: 0,
        },
    );
    take(&mut call, &mut s, sc(11, 1011, 0x2c, &[]), Reply::Word(1));
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 12;
    take(&mut call, &mut s, sc(12, 1012, 0x10, &[]), Reply::Done);
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 13;
    let dispose = sc(13, 1013, 0, &[1]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(dispose.clone()));
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 11;
    call = saved(call);
    call.acknowledge(&dispose, Reply::Done).unwrap();
    finish(&mut call, &mut s, 1);
    assert_eq!(s.geometry.layout.window(1).unwrap().surface, 0);
}
#[test]
fn buffer_failure_disposes_current_owner_and_returns_false_after_notifications() {
    let mut s = state();
    let mut call = Invocation::new(
        1,
        Function::Ownership {
            enabled: 1,
            option: 256,
        },
    );
    take(&mut call, &mut s, sc(11, 1011, 0x2c, &[]), Reply::Word(0));
    take(
        &mut call,
        &mut s,
        Request::BackendFactory { backend: 20 },
        Reply::Word(21),
    );
    take(
        &mut call,
        &mut s,
        Request::FactoryFormat { factory: 21 },
        Reply::Word(30),
    );
    take(
        &mut call,
        &mut s,
        sc(11, 1011, 0xc, &[10, 20, 16, 565, 0, 0, 0]),
        Reply::Word(256),
    );
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 12;
    take(&mut call, &mut s, sc(12, 1012, 0, &[1]), Reply::Done);
    s.geometry.layout.windows.get_mut(&1).unwrap().surface = 13;
    finish(&mut call, &mut s, 0);
    assert_eq!(s.geometry.layout.window(1).unwrap().surface, 0);
}
#[test]
fn binding_preserves_callback_write_without_ancestor_and_overwrites_with_found_surface() {
    for parent in [0, 2] {
        let mut s = state();
        let mut call = Invocation::new(1, Function::Bind);
        take(&mut call, &mut s, wc(1, 0x184, &[]), Reply::Word(0));
        let r = wc(1, 0x18, &[]);
        assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
        assert_eq!(s.drawing[&1], 0);
        s.drawing.insert(1, 777);
        call.acknowledge(&r, Reply::Word(parent)).unwrap();
        if parent != 0 {
            take(&mut call, &mut s, wc(2, 0x184, &[]), Reply::Word(256));
        }
        take(&mut call, &mut s, wc(1, 0x198, &[]), Reply::Done);
        assert_eq!(call.advance(&mut s).unwrap(), None);
        assert_eq!(s.drawing[&1], if parent == 0 { 777 } else { 256 });
    }
}
#[test]
fn binding_propagation_reads_children_after_self_callback_and_rejects_wrong_reply() {
    let mut s = state();
    let mut call = Invocation::new(1, Function::RefreshBindings);
    let r = wc(1, 0x18c, &[]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
    let before = call.clone();
    assert!(call.acknowledge(&r, Reply::Word(1)).is_err());
    assert_eq!(call, before);
    s.geometry.layout.links.get_mut(&101).unwrap().next = 201;
    s.geometry.layout.links.insert(
        201,
        layout::Link {
            next: 101,
            window: 2,
        },
    );
    call.acknowledge(&r, Reply::Done).unwrap();
    take(&mut call, &mut s, wc(2, 0x190, &[]), Reply::Done);
    s.geometry.layout.links.get_mut(&201).unwrap().next = 202;
    s.geometry.layout.links.insert(
        202,
        layout::Link {
            next: 104,
            window: 3,
        },
    );
    s.geometry.layout.windows.get_mut(&1).unwrap().sentinel = 104;
    take(&mut call, &mut s, wc(3, 0x190, &[]), Reply::Done);
    assert_eq!(call.advance(&mut s).unwrap(), None);
    let mut v = json!(Invocation::new(1, Function::Bind));
    v["stage"] = json!("DisposeFailure");
    let mut bad: Invocation = serde_json::from_value(v).unwrap();
    let before = s.clone();
    assert!(bad.advance(&mut s).is_err());
    assert_eq!(s, before);
}

#[test]
fn ownership_composes_inherited_binding_and_geometry_through_saved_nested_frames() {
    use sims_vm::ui_effect::widgets::window;
    #[derive(Clone, Serialize, serde::Deserialize)]
    enum Frame {
        Resource(Invocation),
        Layout(layout::Invocation),
        Geometry(geom::Invocation),
    }
    enum Work {
        Resource(Request),
        Layout(layout::Request),
        Geometry(geom::Request),
    }
    for enabled in [0, 1] {
        let mut s = state();
        s.geometry.layout.windows.get_mut(&4).unwrap().surface = 12;
        s.geometry.layout.links.get_mut(&101).unwrap().next = 201;
        s.geometry.layout.links.insert(
            201,
            layout::Link {
                next: 101,
                window: 2,
            },
        );
        let mut base = window::State {
            global_manager: 9,
            managers: BTreeMap::new(),
            windows: (1..=4)
                .map(|id| {
                    (
                        id,
                        window::Fields {
                            parent: match id {
                                1 => 4,
                                2 => 1,
                                _ => 0,
                            },
                            flags: 1,
                            dirty: 0,
                        },
                    )
                })
                .collect(),
        };
        let mut stack = vec![Frame::Resource(Invocation::new(
            1,
            Function::Ownership { enabled, option: 0 },
        ))];
        let mut result = None;
        let mut backend_calls = vec![];
        while !stack.is_empty() {
            (stack, s, base) = saved((stack, s, base));
            let work = match stack.last_mut().unwrap() {
                Frame::Resource(c) => c.advance(&mut s).unwrap().map(Work::Resource),
                Frame::Layout(c) => c.advance(&mut s.geometry.layout).unwrap().map(Work::Layout),
                Frame::Geometry(c) => c.advance(&mut s.geometry).unwrap().map(Work::Geometry),
            };
            let Some(work) = work else {
                let reply = match stack.pop().unwrap() {
                    Frame::Resource(c) => c.result(),
                    Frame::Geometry(c) => c.result(),
                    Frame::Layout(_) => None,
                };
                if let Some(parent) = stack.last_mut() {
                    match parent {
                        Frame::Resource(c) => {
                            let r = c.request().unwrap().clone();
                            c.acknowledge(&r, Reply::Done).unwrap();
                        }
                        Frame::Layout(c) => {
                            let r = c.request().unwrap().clone();
                            c.acknowledge(&r, layout::Reply::Done).unwrap();
                        }
                        Frame::Geometry(c) => {
                            let r = c.request().unwrap().clone();
                            let a = if matches!(r, geom::Request::Relative { .. }) {
                                geom::Reply::Relative(reply.unwrap())
                            } else {
                                geom::Reply::Done
                            };
                            c.acknowledge(&r, a).unwrap();
                        }
                    }
                } else {
                    result = reply;
                }
                continue;
            };
            let window = match &work {
                Work::Resource(Request::Window(c))
                | Work::Layout(layout::Request::Virtual(c))
                | Work::Geometry(geom::Request::Virtual(c)) => Some(c.clone()),
                Work::Geometry(geom::Request::Relative { window }) => {
                    stack.push(Frame::Geometry(geom::Invocation::new(
                        *window,
                        geom::Function::Relative,
                    )));
                    continue;
                }
                _ => None,
            };
            let word = if let Some(c) = window {
                let child = match c.slot {
                    0x190 => Some(Frame::Resource(Invocation::new(
                        c.window,
                        Function::RefreshBindings,
                    ))),
                    0x18c => Some(Frame::Resource(Invocation::new(c.window, Function::Bind))),
                    0x198 => Some(Frame::Layout(layout::Invocation::new(
                        c.window,
                        layout::Function::Refresh,
                    ))),
                    0x194 => Some(Frame::Geometry(geom::Invocation::new(
                        c.window,
                        geom::Function::Local,
                    ))),
                    _ => None,
                };
                if let Some(child) = child {
                    stack.push(child);
                    continue;
                }
                match c.slot {
                    0x18 => Some(window::parent(base.window(c.window).unwrap())),
                    0x184 => Some(geom::surface(s.geometry.layout.window(c.window).unwrap())),
                    0x8c => Some(window::flags(
                        base.window(c.window).unwrap(),
                        c.arguments[0],
                    )),
                    0x168 => {
                        window::invalidate(base.windows.get_mut(&c.window).unwrap());
                        None
                    }
                    _ => panic!("unimplemented window child {c:?}"),
                }
            } else {
                let Work::Resource(r) = &work else {
                    panic!("unexpected geometry child")
                };
                backend_calls.push(r.clone());
                // Backend methods remain supplied services. All window binding
                // and geometry work below them runs its concrete implementation.
                match r {
                    Request::BackendFactory { .. } => Some(21),
                    Request::FactoryFormat { .. } => Some(30),
                    Request::Surface { slot: 0x2c, .. } => Some(u32::from(enabled == 0)),
                    Request::Surface {
                        slot: 0xc,
                        owner,
                        arguments,
                        ..
                    } => {
                        s.resources.get_mut(owner).unwrap().size =
                            [arguments[0] as i32, arguments[1] as i32];
                        Some(1)
                    }
                    Request::Surface { slot: 0x10, .. } => None,
                    Request::Surface { slot: 0, owner, .. } => {
                        s.resources.remove(owner);
                        None
                    }
                    _ => panic!("unexpected backend service {r:?}"),
                }
            };
            match (stack.last_mut().unwrap(), work) {
                (Frame::Resource(c), Work::Resource(r)) => c
                    .acknowledge(&r, word.map_or(Reply::Done, Reply::Word))
                    .unwrap(),
                (Frame::Layout(c), Work::Layout(r)) => c
                    .acknowledge(&r, word.map_or(layout::Reply::Done, layout::Reply::Word))
                    .unwrap(),
                (Frame::Geometry(c), Work::Geometry(r)) => c
                    .acknowledge(&r, word.map_or(geom::Reply::Done, geom::Reply::Word))
                    .unwrap(),
                _ => unreachable!(),
            }
        }
        assert_eq!(result, Some(1));
        assert_eq!(base.window(1).unwrap().dirty, 1);
        assert_eq!(
            s.geometry.layout.window(1).unwrap().surface,
            if enabled == 0 { 0 } else { 11 }
        );
        assert_eq!(s.drawing[&1], if enabled == 0 { 12 } else { 11 });
        assert_eq!(s.drawing[&2], s.drawing[&1]);
        assert_eq!(
            s.geometry.geometry[&1].local,
            if enabled == 0 {
                [1, 2, 11, 22]
            } else {
                [0, 0, 10, 20]
            }
        );
        assert_eq!(
            s.geometry.geometry[&2].local,
            if enabled == 0 {
                [2, 4, 12, 24]
            } else {
                [1, 2, 11, 22]
            }
        );
        assert_eq!(backend_calls.len(), if enabled == 0 { 3 } else { 4 });
    }
}

fn merge(to: &mut Value, p: &Value) {
    if let Some(o) = p.as_object() {
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
    let word = match r {
        Request::BackendFactory { .. } => Some(c["factory_reply"].as_u64().unwrap() as u32),
        Request::FactoryCreate { .. } => Some(c["resource_reply"].as_u64().unwrap() as u32),
        Request::FactoryFormat { .. } => Some(c["format_reply"].as_u64().unwrap() as u32),
        Request::Palette { .. } => Some(c["palette_reply"].as_u64().unwrap() as u32),
        Request::Surface { slot: 0x2c, .. } => Some(c["ready_reply"].as_u64().unwrap() as u32),
        Request::Surface { slot: 0xc, .. } => Some(c["buffer_reply"].as_u64().unwrap() as u32),
        Request::Window(v) if v.slot == 0x8c => Some(c["flag_reply"].as_u64().unwrap() as u32),
        Request::Window(v) if v.slot == 0x18 => {
            Some(c["parents"][v.window.to_string()].as_u64().unwrap_or(0) as u32)
        }
        Request::Window(v) if v.slot == 0x184 => {
            Some(s.geometry.layout.window(v.window).unwrap().surface)
        }
        _ => None,
    };
    word.map_or(Reply::Done, Reply::Word)
}
fn run(c: &Value, restore: bool) -> Value {
    let mut s: State = serde_json::from_value(c["state"].clone()).unwrap();
    if c["function"] == "PaletteOwner" {
        return json!({"state":s,"events":[],"result":res::palette_owner()});
    }
    let function: Function = serde_json::from_value(c["function"].clone()).unwrap();
    let recursive = function == Function::RefreshBindings;
    let mut stack = vec![Invocation::new(
        c["owner"].as_u64().unwrap() as u32,
        function,
    )];
    let mut events = vec![];
    let mut result = None;
    while !stack.is_empty() {
        if restore {
            (stack, s) = saved((stack, s));
        }
        let Some(r) = stack.last_mut().unwrap().advance(&mut s).unwrap() else {
            let done = stack.pop().unwrap();
            if let Some(p) = stack.last_mut() {
                let r = p.request().unwrap().clone();
                p.acknowledge(&r, Reply::Done).unwrap();
            } else {
                result = done.result();
            }
            continue;
        };
        if recursive {
            if let Request::Window(v) = &r {
                let f = match v.slot {
                    0x190 => Some(Function::RefreshBindings),
                    0x18c => Some(Function::Bind),
                    _ => None,
                };
                if let Some(f) = f {
                    stack.push(Invocation::new(v.window, f));
                    continue;
                }
            }
        }
        let n = events.len().to_string();
        let reply = c["replies"]
            .get(&n)
            .cloned()
            .map(|x| serde_json::from_value(x).unwrap())
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
    json!({"state":s,"events":events,"result":result})
}
fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[test]
#[ignore = "private Rust surface resource matrix; no original execution"]
fn prepared_window_resource_cases_restore_every_callback() {
    let all: Value = serde_json::from_slice(
        &std::fs::read(root().join("cache/native-window-resource-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(all["cases"].as_array().unwrap().len(), 742);
    for (n, c) in all["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case{n}");
    }
}
#[test]
#[ignore = "guarded original allocation/binding caller comparison"]
fn window_resource_calls_match_original() {
    let all: Value = serde_json::from_slice(
        &std::fs::read(root().join("cache/research/sims-exe/oracle/window-resource.json")).unwrap(),
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
    assert_eq!(all["cases"].as_array().unwrap().len(), 742);
    for (n, c) in all["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(&c["input"], true), c["original"], "case{n}");
    }
}
