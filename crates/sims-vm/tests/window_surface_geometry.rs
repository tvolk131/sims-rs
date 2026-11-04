use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::ui_effect::widgets::{
    layout,
    surface_geometry::{self as geom, Fields, Function, Invocation, Region, Reply, Request, State},
    visibility::Call,
};
use std::collections::BTreeMap;
fn saved<T: Serialize + DeserializeOwned>(v: T) -> T {
    serde_json::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap()
}
fn state() -> State {
    State {
        layout: layout::State {
            windows: (1..=4)
                .map(|id| {
                    (
                        id,
                        layout::Fields {
                            rectangle: [id as i32, 2, 11, 22],
                            absolute: [0, 0, 100, 100],
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
                    Fields {
                        local: [-1; 4],
                        relative: [-2; 4],
                    },
                )
            })
            .collect(),
        regions: BTreeMap::from([(10, [0, 0, 50, 50])]),
    }
}
fn vc(window: u32, slot: u16, args: &[u32]) -> Request {
    Request::Virtual(Call::new(window, slot, args))
}
fn take(call: &mut Invocation, s: &mut State, r: Request, reply: Reply) {
    assert_eq!(call.advance(s).unwrap(), Some(r.clone()));
    *call = saved(call.clone());
    assert_eq!(call.advance(s).unwrap(), Some(r.clone()));
    call.acknowledge(&r, reply).unwrap();
    *call = saved(call.clone());
}
#[test]
fn local_without_parent_retains_callback_write_but_nonnull_parent_commits_captured_rectangle() {
    for parent in [0, 2] {
        let mut s = state();
        let mut call = Invocation::new(1, Function::Local);
        let r = vc(1, 0x18, &[]);
        assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
        assert_eq!(s.geometry[&1].local, [1, 2, 11, 22]);
        s.geometry.get_mut(&1).unwrap().local = [99; 4];
        s.layout.windows.get_mut(&1).unwrap().rectangle = [88; 4];
        call.acknowledge(&r, Reply::Word(parent)).unwrap();
        if parent != 0 {
            take(&mut call, &mut s, vc(2, 0x184, &[]), Reply::Word(256));
        }
        assert_eq!(call.advance(&mut s).unwrap(), None);
        assert_eq!(
            s.geometry[&1].local,
            if parent == 0 { [99; 4] } else { [1, 2, 11, 22] }
        );
    }
}
#[test]
fn local_uses_full_surface_words_and_reads_parent_offsets_after_callbacks() {
    let mut s = state();
    let mut call = Invocation::new(1, Function::Local);
    take(&mut call, &mut s, vc(1, 0x18, &[]), Reply::Word(2));
    take(&mut call, &mut s, vc(2, 0x184, &[]), Reply::Word(0));
    s.layout.windows.get_mut(&2).unwrap().rectangle = [10, 20, 100, 100];
    take(&mut call, &mut s, vc(2, 0x18, &[]), Reply::Word(3));
    s.geometry.get_mut(&1).unwrap().local = [99; 4];
    s.layout.windows.get_mut(&2).unwrap().rectangle = [88; 4];
    take(&mut call, &mut s, vc(3, 0x184, &[]), Reply::Word(0x100));
    assert_eq!(call.advance(&mut s).unwrap(), None);
    assert_eq!(s.geometry[&1].local, [11, 22, 21, 42]);
}
#[test]
fn local_surface_branch_writes_size_before_relative_child_and_keeps_child_result_out_of_state() {
    let mut s = state();
    s.layout.windows.get_mut(&1).unwrap().surface = 256;
    s.layout.windows.get_mut(&1).unwrap().rectangle = [i32::MIN, 10, i32::MAX, -20];
    let mut call = Invocation::new(1, Function::Local);
    let r = Request::Relative { window: 1 };
    assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
    assert_eq!(s.geometry[&1].local, [0, 0, -1, -30]);
    let before = call.clone();
    assert!(call.acknowledge(&r, Reply::Done).is_err());
    assert_eq!(call, before);
    call.acknowledge(&r, Reply::Relative(1)).unwrap();
    assert_eq!(call.advance(&mut s).unwrap(), None);
    assert_eq!(call.result(), None);
}
#[test]
fn relative_reads_self_after_parent_query_and_keeps_shared_callback_mutations() {
    let mut s = state();
    let mut call = Invocation::new(1, Function::Relative);
    take(&mut call, &mut s, vc(1, 0x184, &[]), Reply::Word(256));
    take(&mut call, &mut s, vc(1, 0x18, &[]), Reply::Word(2));
    s.layout.windows.get_mut(&1).unwrap().rectangle = [5, 6, 15, 26];
    let r = vc(2, 0x184, &[]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
    assert_eq!(s.geometry[&1].relative, [5, 6, 15, 26]);
    s.geometry.get_mut(&1).unwrap().relative = [100, 200, 300, 400];
    s.layout.windows.get_mut(&2).unwrap().rectangle = [10, 20, 0, 0];
    call.acknowledge(&r, Reply::Word(0)).unwrap();
    let parent = vc(2, 0x18, &[]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(parent.clone()));
    assert_eq!(s.geometry[&1].relative, [110, 220, 310, 420]);
    s.geometry.get_mut(&1).unwrap().relative = [77; 4];
    call = saved(call);
    call.acknowledge(&parent, Reply::Word(3)).unwrap();
    take(&mut call, &mut s, vc(3, 0x184, &[]), Reply::Word(1));
    assert_eq!(call.advance(&mut s).unwrap(), None);
    assert_eq!(call.result(), Some(1));
    assert_eq!(s.geometry[&1].relative, [77; 4]);
}
#[test]
fn relative_without_surface_does_not_write_but_without_parent_copies_rectangle_and_returns_false() {
    for surface in [0, 256] {
        let mut s = state();
        let mut call = Invocation::new(1, Function::Relative);
        take(&mut call, &mut s, vc(1, 0x184, &[]), Reply::Word(surface));
        if surface != 0 {
            take(&mut call, &mut s, vc(1, 0x18, &[]), Reply::Word(0));
        }
        assert_eq!(call.advance(&mut s).unwrap(), None);
        assert_eq!(call.result(), Some(0));
        assert_eq!(
            s.geometry[&1].relative,
            if surface == 0 {
                [-2; 4]
            } else {
                [1, 2, 11, 22]
            }
        );
    }
}
#[test]
fn clip_honors_low_byte_and_zero_intersection_equality_before_traversing() {
    for (flag, absolute, dirty) in [
        (256, [0, 0, 100, 100], false),
        (257, [0, 0, 100, 100], true),
        (1, [0, 0, 20, 20], false),
        (1, [0; 4], false),
    ] {
        let mut s = state();
        s.layout.windows.get_mut(&1).unwrap().absolute = absolute;
        let mut call = Invocation::new(
            1,
            Function::Clip {
                region: Region::External(10),
            },
        );
        take(&mut call, &mut s, vc(1, 0x8c, &[1]), Reply::Word(flag));
        if dirty {
            take(&mut call, &mut s, vc(1, 0x168, &[]), Reply::Done);
        }
        assert_eq!(call.advance(&mut s).unwrap(), None);
    }
    assert_eq!(geom::intersection([0, 0, 5, 5], [5, 0, 10, 5]), ([0; 4], 0));
    assert_eq!(
        geom::intersection([4, 4, 2, 2], [0, 0, 6, 6]),
        ([4, 4, 2, 2], 1)
    );
}
#[test]
fn clip_preserves_region_reference_and_rereads_list_after_invalidation_and_child() {
    let mut s = state();
    let region = Region::Window(4);
    s.layout.windows.get_mut(&4).unwrap().absolute = [0, 0, 50, 50];
    let mut call = Invocation::new(1, Function::Clip { region });
    take(&mut call, &mut s, vc(1, 0x8c, &[1]), Reply::Word(1));
    take(&mut call, &mut s, vc(1, 0x168, &[]), Reply::Done);
    s.layout.links.get_mut(&101).unwrap().next = 201;
    s.layout.links.insert(
        201,
        layout::Link {
            next: 101,
            window: 2,
        },
    );
    let child = Request::Clip { window: 2, region };
    assert_eq!(call.advance(&mut s).unwrap(), Some(child.clone()));
    s.layout.windows.get_mut(&4).unwrap().absolute = [0, 0, 200, 200];
    let mut nested = Invocation::new(2, Function::Clip { region });
    take(&mut nested, &mut s, vc(2, 0x8c, &[1]), Reply::Word(1));
    assert_eq!(nested.advance(&mut s).unwrap(), None); //new live region now contains child
    s.layout.links.get_mut(&201).unwrap().next = 202;
    s.layout.links.insert(
        202,
        layout::Link {
            next: 104,
            window: 3,
        },
    );
    s.layout.windows.get_mut(&1).unwrap().sentinel = 104;
    call = saved(call);
    call.acknowledge(&child, Reply::Done).unwrap();
    take(
        &mut call,
        &mut s,
        Request::Clip { window: 3, region },
        Reply::Done,
    );
    assert_eq!(call.advance(&mut s).unwrap(), None);
}
#[test]
fn corrupt_surface_continuations_reject_before_mutating_geometry() {
    let mut s = state();
    let before = s.clone();
    let mut v = json!(Invocation::new(1, Function::Local));
    v["stage"] = json!("RelativeParentSurface");
    let mut call: Invocation = serde_json::from_value(v).unwrap();
    assert!(call.advance(&mut s).is_err());
    assert_eq!(s, before);
    let mut call = Invocation::new(1, Function::Relative);
    let r = vc(1, 0x184, &[]);
    assert_eq!(call.advance(&mut s).unwrap(), Some(r.clone()));
    let before_call = call.clone();
    assert!(call.acknowledge(&r, Reply::Relative(1)).is_err());
    assert_eq!(call, before_call);
    assert_eq!(s, before);
}

#[test]
fn move_composes_surface_positioning_absolute_layout_and_root_clipping_without_placeholder_children()
 {
    use sims_vm::ui_effect::widgets::window;
    #[derive(Clone, Serialize, serde::Deserialize)]
    enum Frame {
        Layout(layout::Invocation),
        Surface(Invocation),
    }
    enum Work {
        Layout(layout::Request),
        Surface(Request),
    }
    let mut s = state();
    s.layout.links.get_mut(&101).unwrap().next = 201;
    s.layout.links.insert(
        201,
        layout::Link {
            next: 202,
            window: 2,
        },
    );
    s.layout.links.insert(
        202,
        layout::Link {
            next: 101,
            window: 3,
        },
    );
    s.layout.links.get_mut(&104).unwrap().next = 204;
    s.layout.links.insert(
        204,
        layout::Link {
            next: 104,
            window: 1,
        },
    );
    s.layout.windows.get_mut(&2).unwrap().surface = 512;
    s.layout.windows.get_mut(&4).unwrap().surface = 256;
    let parents = BTreeMap::from([(1, 4), (2, 1), (3, 1), (4, 0)]);
    let mut base = window::State {
        global_manager: 9,
        managers: BTreeMap::new(),
        windows: (1..=4)
            .map(|id| {
                (
                    id,
                    window::Fields {
                        parent: parents[&id],
                        flags: if id == 1 { 9 } else { 1 },
                        dirty: 0,
                    },
                )
            })
            .collect(),
    };
    let mut stack = vec![Frame::Layout(layout::Invocation::new(
        1,
        layout::Function::Move { x: 10, y: 20 },
    ))];
    while !stack.is_empty() {
        (stack, s, base) = saved((stack, s, base));
        let work = match stack.last_mut().unwrap() {
            Frame::Layout(call) => call.advance(&mut s.layout).unwrap().map(Work::Layout),
            Frame::Surface(call) => call.advance(&mut s).unwrap().map(Work::Surface),
        };
        let Some(work) = work else {
            let result = match stack.pop().unwrap() {
                Frame::Surface(call) => call.result(),
                Frame::Layout(_) => None,
            };
            if let Some(parent) = stack.last_mut() {
                match parent {
                    Frame::Layout(call) => {
                        let r = call.request().unwrap().clone();
                        call.acknowledge(&r, layout::Reply::Done).unwrap();
                    }
                    Frame::Surface(call) => {
                        let r = call.request().unwrap().clone();
                        let reply = if matches!(r, Request::Relative { .. }) {
                            Reply::Relative(result.unwrap())
                        } else {
                            Reply::Done
                        };
                        call.acknowledge(&r, reply).unwrap();
                    }
                }
            }
            continue;
        };
        let (owner, slot, args) = match &work {
            Work::Layout(layout::Request::Virtual(c)) | Work::Surface(Request::Virtual(c)) => {
                (c.window, c.slot, c.arguments.clone())
            }
            Work::Layout(layout::Request::RootRegion { root, window }) => {
                stack.push(Frame::Surface(Invocation::new(
                    *root,
                    Function::Clip {
                        region: Region::Window(*window),
                    },
                )));
                continue;
            }
            Work::Surface(Request::Relative { window }) => {
                stack.push(Frame::Surface(Invocation::new(*window, Function::Relative)));
                continue;
            }
            Work::Surface(Request::Clip { window, region }) => {
                stack.push(Frame::Surface(Invocation::new(
                    *window,
                    Function::Clip { region: *region },
                )));
                continue;
            }
            _ => panic!("unsupported resource child reached by composition"),
        };
        let child = match slot {
            0x60 => Some(Frame::Layout(layout::Invocation::new(
                owner,
                layout::Function::Rectangle {
                    rectangle: args
                        .iter()
                        .map(|x| *x as i32)
                        .collect::<Vec<_>>()
                        .try_into()
                        .unwrap(),
                },
            ))),
            0x198 => Some(Frame::Layout(layout::Invocation::new(
                owner,
                layout::Function::Refresh,
            ))),
            0x88 => Some(Frame::Layout(layout::Invocation::new(
                owner,
                layout::Function::Absolute,
            ))),
            0x194 => Some(Frame::Surface(Invocation::new(owner, Function::Local))),
            _ => None,
        };
        if let Some(child) = child {
            stack.push(child);
            continue;
        }
        let word = match slot {
            0x18 => Some(window::parent(base.window(owner).unwrap())),
            0x8c => Some(window::flags(base.window(owner).unwrap(), args[0])),
            0x184 => Some(geom::surface(s.layout.window(owner).unwrap())),
            0x168 => {
                window::invalidate(base.windows.get_mut(&owner).unwrap());
                None
            }
            _ => panic!("unimplemented virtual child {slot:x}"),
        };
        match (stack.last_mut().unwrap(), work) {
            (Frame::Layout(call), Work::Layout(r)) => call
                .acknowledge(&r, word.map_or(layout::Reply::Done, layout::Reply::Word))
                .unwrap(),
            (Frame::Surface(call), Work::Surface(r)) => call
                .acknowledge(&r, word.map_or(Reply::Done, Reply::Word))
                .unwrap(),
            _ => unreachable!(),
        }
    }
    assert_eq!(s.geometry[&1].local, [10, 20, 20, 40]);
    assert_eq!(s.geometry[&2].local, [0, 0, 9, 20]);
    assert_eq!(s.geometry[&2].relative, [12, 22, 21, 42]);
    assert_eq!(s.geometry[&3].local, [13, 22, 21, 42]);
    assert_eq!(s.layout.window(1).unwrap().absolute, [14, 22, 24, 42]);
    assert_eq!(s.layout.window(2).unwrap().absolute, [16, 24, 25, 44]);
    assert_eq!(base.window(4).unwrap().dirty, 1);
    assert!(
        [1, 2, 3]
            .iter()
            .all(|id| base.window(*id).unwrap().dirty == 0)
    );
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
    match r {
        Request::Virtual(v) if v.slot == 0x18 => {
            Reply::Word(c["parents"][v.window.to_string()].as_u64().unwrap_or(0) as u32)
        }
        Request::Virtual(v) if v.slot == 0x184 => {
            Reply::Word(geom::surface(s.layout.window(v.window).unwrap()))
        }
        Request::Virtual(v) if v.slot == 0x8c => {
            Reply::Word(c["flag_reply"].as_u64().unwrap() as u32)
        }
        Request::Virtual(v) if v.slot == 0x168 => Reply::Done,
        _ => panic!("unexpected supplied service {r:?}"),
    }
}
fn run(c: &Value, restore: bool) -> Value {
    let mut s: State = serde_json::from_value(c["state"].clone()).unwrap();
    let owner = c["owner"].as_u64().unwrap() as u32;
    if c["function"] == "Surface" {
        return json!({"state":s,"events":[],"result":geom::surface(s.layout.window(owner).unwrap())});
    }
    if let Some(x) = c["function"].get("Intersection") {
        let a = serde_json::from_value(x["a"].clone()).unwrap();
        let b = serde_json::from_value(x["b"].clone()).unwrap();
        let (r, v) = geom::intersection(a, b);
        return json!({"rectangle":r,"result":v});
    }
    let mut stack = vec![Invocation::new(
        owner,
        serde_json::from_value(c["function"].clone()).unwrap(),
    )];
    let mut events = vec![];
    let mut result = None;
    while !stack.is_empty() {
        if restore {
            (stack, s) = saved((stack, s));
        }
        let Some(r) = stack.last_mut().unwrap().advance(&mut s).unwrap() else {
            let done = stack.pop().unwrap();
            if let Some(parent) = stack.last_mut() {
                let r = parent.request().unwrap().clone();
                let reply = if matches!(r, Request::Relative { .. }) {
                    Reply::Relative(done.result().unwrap())
                } else {
                    Reply::Done
                };
                parent.acknowledge(&r, reply).unwrap();
            } else {
                result = done.result();
            }
            continue;
        };
        match &r {
            Request::Relative { window } => {
                stack.push(Invocation::new(*window, Function::Relative));
                continue;
            }
            Request::Clip { window, region } => {
                stack.push(Invocation::new(*window, Function::Clip { region: *region }));
                continue;
            }
            _ => {}
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
#[ignore = "private Rust surface geometry continuation matrix; no original execution"]
fn prepared_window_surface_cases_restore_each_callback() {
    let all: Value = serde_json::from_slice(
        &std::fs::read(root().join("cache/native-window-surface-inputs.json")).unwrap(),
    )
    .unwrap();
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1806);
    for (n, c) in cases.iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case{n}");
    }
}
#[test]
#[ignore = "guarded original surface geometry/clip/recursive child comparison"]
fn window_surface_geometry_matches_original() {
    let all: Value = serde_json::from_slice(
        &std::fs::read(root().join("cache/research/sims-exe/oracle/window-surface.json")).unwrap(),
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
    assert_eq!(all["cases"].as_array().unwrap().len(), 1806);
    for (n, c) in all["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(&c["input"], true), c["original"], "case{n}");
    }
}
