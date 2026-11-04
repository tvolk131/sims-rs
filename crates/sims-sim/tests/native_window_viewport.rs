use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_sim::lot_presenter::viewport::{
    self, Function, Invocation, Renderer, Reply, Request, State, View, Window,
};
use std::collections::BTreeMap;
fn saved<T: Serialize + DeserializeOwned>(v: T) -> T {
    serde_json::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap()
}
fn state() -> State {
    State {
        views: BTreeMap::from([(
            1,
            View {
                window: 2,
                renderer: 3,
                surface: 4,
                rectangle: [0, 0, 800, 480],
                retained: [1; 4],
            },
        )]),
        renderers: BTreeMap::from([(
            3,
            Renderer {
                window: 2,
                width: 800,
            },
        )]),
        windows: BTreeMap::from([(
            2,
            Window {
                origin: [0, 0],
                size: [800, 480],
                scale: [0; 4],
                rectangle: [0, 0, 800, 480],
                backend: 5,
                result: 99,
            },
        )]),
        surfaces: BTreeMap::from([(4, [800, 600])]),
        targets: BTreeMap::from([(6, [[1; 4]; 2]), (7, [[2; 4]; 2])]),
        target: 6,
    }
}
fn take(c: &mut Invocation, s: &mut State, r: Request, reply: Reply) {
    assert_eq!(c.advance(s).unwrap(), Some(r));
    (*c, *s) = saved((c.clone(), s.clone()));
    assert_eq!(c.advance(s).unwrap(), Some(r));
    let wrong = if reply == Reply::Done {
        Reply::Renderer(0)
    } else {
        Reply::Done
    };
    let before = c.clone();
    assert!(c.acknowledge(r, wrong).is_err());
    assert_eq!(*c, before);
    c.acknowledge(r, reply).unwrap();
    *c = saved(c.clone());
}
#[test]
fn resize_uses_wrapping_edges_and_unsigned_dimensions_before_backend_call() {
    let mut s = state();
    s.windows.get_mut(&2).unwrap().origin = [i32::MAX, i32::MIN];
    let mut c = Invocation::new(2, Function::Resize([i32::MIN, -1]));
    let r = Request::Backend {
        backend: 5,
        window: 2,
    };
    assert_eq!(c.advance(&mut s).unwrap(), Some(r));
    assert_eq!(s.windows[&2].rectangle, [i32::MAX, i32::MIN, -1, i32::MAX]);
    assert_eq!(
        s.windows[&2].scale,
        [0x4e800000, 0x4f000000, 0x3f800000, 0x3f800000]
    );
    s.windows.get_mut(&2).unwrap().backend = 8;
    s.windows.get_mut(&2).unwrap().result = 123;
    s.windows.get_mut(&2).unwrap().size = [3, 4];
    take(&mut c, &mut s, r, Reply::Backend(0));
    assert_eq!(c.advance(&mut s).unwrap(), None);
    assert_eq!(s.windows[&2].result, 0);
    assert_eq!(s.windows[&2].size, [3, 4]);
    assert_eq!(s.windows[&2].backend, 8);
}
#[test]
fn renderer_height_sign_extends_width_and_retains_resize_receiver() {
    let mut s = state();
    s.renderers.get_mut(&3).unwrap().width = -1;
    let mut c = Invocation::new(3, Function::RendererHeight(600));
    let r = Request::Resize {
        window: 2,
        size: [-1, 600],
    };
    assert_eq!(c.advance(&mut s).unwrap(), Some(r));
    s.renderers.get_mut(&3).unwrap().window = 9;
    take(&mut c, &mut s, r, Reply::Done);
    assert_eq!(c.advance(&mut s).unwrap(), None);
    assert!(c.complete());
}
#[test]
fn viewport_retains_old_height_but_rereads_window_surface_and_target_after_callbacks() {
    let mut s = state();
    let mut c = Invocation::new(1, Function::Height(600));
    take(&mut c, &mut s, Request::Renderer(1), Reply::Renderer(30));
    s.windows.get_mut(&2).unwrap().rectangle = [0, 0, 100, 99];
    take(
        &mut c,
        &mut s,
        Request::RendererHeight {
            renderer: 30,
            height: 600,
        },
        Reply::Done,
    );
    // The height child can replace the current window, not only its geometry.
    let mut other = s.windows[&2].clone();
    other.rectangle = [5, 7, 105, 307];
    s.windows.insert(20, other);
    s.views.get_mut(&1).unwrap().window = 20;
    let r = Request::Synchronize(1);
    assert_eq!(c.advance(&mut s).unwrap(), Some(r));
    assert_eq!(s.views[&1].rectangle, [0, 0, 100, 300]);
    s.views.get_mut(&1).unwrap().rectangle = [9, 10, 777, 888];
    s.views.get_mut(&1).unwrap().surface = 40;
    s.surfaces.insert(40, [321, 600]);
    s.target = 7;
    take(&mut c, &mut s, r, Reply::Done);
    assert_eq!(c.advance(&mut s).unwrap(), None);
    assert_eq!(s.views[&1].retained, [0, 120, 321, 600]);
    assert_eq!(s.targets[&6], [[1; 4]; 2]);
    assert_eq!(s.targets[&7], [[0, 0, 777, 600], [0, 600, 777, 1200]]);
}
#[test]
fn nonmatching_surface_clears_retained_strip_and_target_bottom_wraps() {
    let mut s = state();
    let mut c = Invocation::new(1, Function::Height(i32::MAX));
    take(&mut c, &mut s, Request::Renderer(1), Reply::Renderer(3));
    take(
        &mut c,
        &mut s,
        Request::RendererHeight {
            renderer: 3,
            height: i32::MAX,
        },
        Reply::Done,
    );
    take(&mut c, &mut s, Request::Synchronize(1), Reply::Done);
    assert_eq!(c.advance(&mut s).unwrap(), None);
    assert_eq!(s.views[&1].retained, [0; 4]);
    assert_eq!(s.targets[&6][1], [0, i32::MAX, 800, -2]);
}
#[test]
fn absent_target_reports_error_after_original_prior_writes() {
    let mut s = state();
    let mut c = Invocation::new(1, Function::Height(600));
    take(&mut c, &mut s, Request::Renderer(1), Reply::Renderer(3));
    take(
        &mut c,
        &mut s,
        Request::RendererHeight {
            renderer: 3,
            height: 600,
        },
        Reply::Done,
    );
    take(&mut c, &mut s, Request::Synchronize(1), Reply::Done);
    s.target = 0;
    assert!(c.advance(&mut s).is_err());
    assert_eq!(s.views[&1].retained, [0, 120, 800, 600]);
    assert!(!c.complete());
}
#[test]
fn corrupt_continuation_is_rejected_before_geometry_changes() {
    let mut s = state();
    let mut c = Invocation::new(1, Function::Height(600));
    c.advance(&mut s).unwrap();
    let mut v = json!(c);
    v["pending"]["Renderer"] = json!(2);
    let mut bad: Invocation = serde_json::from_value(v).unwrap();
    let before = s.clone();
    assert!(bad.advance(&mut s).is_err());
    assert_eq!(s, before);
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
fn run(c: &Value, restore: bool) -> Value {
    let mut s: State = serde_json::from_value(c["state"].clone()).unwrap();
    let mut stack = vec![Invocation::new(
        c["owner"].as_u64().unwrap() as u32,
        serde_json::from_value(c["function"].clone()).unwrap(),
    )];
    let mut events = vec![];
    while !stack.is_empty() {
        assert!(events.len() < 20);
        let Some(r) = stack.last_mut().unwrap().advance(&mut s).unwrap() else {
            assert!(stack.pop().unwrap().complete());
            if let Some(parent) = stack.last_mut() {
                parent
                    .acknowledge(parent.request().unwrap(), Reply::Done)
                    .unwrap();
            }
            continue;
        };
        if restore {
            (s, stack) = saved((s, stack));
            assert_eq!(stack.last().unwrap().request(), Some(r));
        }
        match r {
            Request::RendererHeight { renderer, height } => {
                stack.push(Invocation::new(renderer, Function::RendererHeight(height)));
                continue;
            }
            Request::Resize { window, size } => {
                stack.push(Invocation::new(window, Function::Resize(size)));
                continue;
            }
            _ => (),
        }
        let n = events.len().to_string();
        events.push(json!({"request":r,"state":s}));
        let reply = if !c["replies"][&n].is_null() {
            serde_json::from_value(c["replies"][&n].clone()).unwrap()
        } else {
            match r {
                Request::Renderer(_) => {
                    Reply::Renderer(c["renderer_reply"].as_u64().unwrap() as u32)
                }
                Request::Backend { .. } => {
                    Reply::Backend(c["backend_reply"].as_u64().unwrap() as u32)
                }
                Request::Synchronize(_) => Reply::Done,
                _ => unreachable!(),
            }
        };
        if !c["patches"][&n].is_null() {
            let mut v = json!(s);
            merge(&mut v, &c["patches"][&n]);
            s = serde_json::from_value(v).unwrap();
        }
        stack.last_mut().unwrap().acknowledge(r, reply).unwrap();
        if restore {
            (s, stack) = saved((s, stack));
        }
    }
    json!({"events":events,"state":s})
}
#[test]
#[ignore = "private prepared inputs; Rust save/resume, not original execution"]
fn prepared_viewport_cases_restore_composed_children_at_every_callback() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../cache/native-window-viewport-inputs.json"
    );
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], false);
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1088);
    for (n, c) in rows.iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "viewport {n}");
    }
}
#[test]
#[ignore = "requires separately executed original instructions"]
fn viewport_matches_original_instructions() {
    let report: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("SIMS_NATIVE_WINDOW_VIEWPORT").unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            run(&row["input"], true),
            row["original"],
            "native viewport {n}"
        );
    }
}
#[test]
fn unsigned_half_scales_round_once_at_f32_storage() {
    assert_eq!(
        viewport::scales([-1, 16777217]),
        [0x4f000000, 0x4b000000, 0x3f800000, 0x3f800000]
    );
    assert_eq!(
        viewport::scales([0, 1]),
        [0, 0x3f000000, 0x3f800000, 0x3f800000]
    );
}
