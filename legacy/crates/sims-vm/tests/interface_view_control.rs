use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_vm::interface::{state::State, view_control::*};
use std::collections::BTreeMap;

const VIEW: u32 = 0x10000000;
const ARG: u32 = VIEW + 0x2000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fixture {
    fields: Fields,
    dirty: u32,
    after_dirty: u32,
    enabled: BTreeMap<u32, u8>,
    selected: BTreeMap<u32, i32>,
    texts: BTreeMap<String, String>,
    queries: BTreeMap<String, u32>,
    now: u32,
    lot: u32,
    panel: u32,
}
fn key(t: Text) -> String {
    format!("{}:{}", t.owner, t.offset)
}
fn source(offset: u16) -> Text {
    Text { owner: ARG, offset }
}
fn fixture(mask: u32) -> Fixture {
    let fields = Fields {
        registered: VIEW + 0x1000,
        widgets: std::array::from_fn(|n| VIEW + 0x5000 + n as u32 * 0x200),
        blink: Blink {
            phase: 0,
            interval: 10,
            deadline: 200,
            last: 200,
            first: 10,
            second: 20,
        },
    };
    Fixture {
        dirty: mask,
        after_dirty: 0x804000,
        now: 100,
        lot: VIEW + 0x3000,
        panel: VIEW + 0x4000,
        enabled: fields.widgets.into_iter().map(|id| (id, 7)).collect(),
        selected: fields.widgets.into_iter().map(|id| (id, 9)).collect(),
        texts: [
            (source(0x28), "new"),
            (source(0x24), "title"),
            (
                Text {
                    owner: fields.widget(Widget::Status),
                    offset: 0x4c,
                },
                "old",
            ),
            (
                Text {
                    owner: VIEW,
                    offset: 0x170,
                },
                "prior title",
            ),
            (
                Text {
                    owner: VIEW,
                    offset: 0x194,
                },
                "buffer",
            ),
            (
                Text {
                    owner: VIEW,
                    offset: 0x1d4,
                },
                "prior name",
            ),
            (
                Text {
                    owner: VIEW + 0x9000,
                    offset: 0,
                },
                "name",
            ),
        ]
        .into_iter()
        .map(|(t, s)| (key(t), s.into()))
        .collect(),
        queries: [
            ("Mode", 2),
            ("Floor", 2),
            ("BuildFloorAvailable", 0),
            ("UpperFloorAvailable", 0),
            ("Walls", 4),
            ("AvailableWalls", 7),
            ("ZoomIn", 128),
            ("ZoomOut", 255),
            ("LotPaused", 128),
            ("SelectionName", VIEW + 0x9000),
            ("SavedSpeedActive", 0),
            ("SavedSpeed", 924),
            ("Speed", 994),
            ("Expansion", 1),
            ("BuyBlocked", 0),
            ("LiveBlocked", 2),
            ("LabelsHidden", 1),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect(),
        fields,
    }
}
fn service(x: &mut Fixture, r: Request) -> Reply {
    match r {
        Request::Get {
            query: Query::Status,
            ..
        } => Reply::Text(source(0x28)),
        Request::Get {
            query: Query::Title,
            ..
        } => Reply::Text(source(0x24)),
        Request::Get { query, .. } => {
            Reply::Number(x.queries[serde_json::to_value(query).unwrap().as_str().unwrap()])
        }
        Request::Length(t) => Reply::Number(x.texts[&key(t)].len() as u32),
        Request::Equal { left, right } => {
            Reply::Number(u32::from(x.texts[&key(left)] == x.texts[&key(right)]))
        }
        Request::Copy { target, source } => {
            x.texts.insert(key(target), x.texts[&key(source)].clone());
            Reply::Done
        }
        Request::SetText { widget, source } => {
            x.texts.insert(
                key(Text {
                    owner: widget,
                    offset: 0x4c,
                }),
                x.texts[&key(source)].clone(),
            );
            Reply::Done
        }
        Request::CopyCString { target, source } => {
            x.texts.insert(
                key(target),
                x.texts[&key(Text {
                    owner: source,
                    offset: 0,
                })]
                    .clone(),
            );
            Reply::Done
        }
        Request::Selected { widget, value } => {
            x.selected.insert(widget, value);
            Reply::Done
        }
        _ => Reply::Done, // Explicit caller boundary; these child bodies are not claimed.
    }
}
fn merge(to: &mut Value, from: &Value) {
    if let (Some(dst), Some(src)) = (to.as_object_mut(), from.as_object()) {
        for (k, v) in src {
            merge(
                dst.get_mut(k)
                    .unwrap_or_else(|| panic!("unknown patch {k}")),
                v,
            );
        }
    } else {
        *to = from.clone();
    }
}
fn run(mut x: Fixture, patches: &BTreeMap<usize, Value>, restore: bool) -> Value {
    let mut s = State::constructed(760);
    let mut call = Update::new(VIEW, ARG);
    let mut events = vec![];
    for n in 0..160 {
        s.buy.dirty = x.dirty;
        s.publication.as_mut().unwrap().after_dirty = x.after_dirty;
        let input = Inputs {
            now: x.now,
            lot: x.lot,
            panel: x.panel,
        };
        let r = call
            .advance(&mut x.fields, &mut s, &mut x.enabled, input)
            .unwrap();
        x.dirty = s.buy.dirty;
        x.after_dirty = s.publication.as_ref().unwrap().after_dirty;
        let Some(r) = r else {
            break;
        };
        call.validate().unwrap();
        events.push(json!({"request":r,"state":x}));
        if restore {
            (x, s, call) =
                serde_json::from_slice(&serde_json::to_vec(&(&x, &s, &call)).unwrap()).unwrap();
            let before = x.clone();
            assert_eq!(
                call.advance(&mut x.fields, &mut s, &mut x.enabled, input)
                    .unwrap(),
                Some(r)
            );
            assert_eq!(x, before);
            let before = call.clone();
            let wrong = match r {
                Request::Invalidate(id) => Request::Invalidate(id + 1),
                _ => Request::Invalidate(0),
            };
            assert!(call.acknowledge(wrong, Reply::Done).is_err());
            assert_eq!(call, before);
        }
        let reply = service(&mut x, r);
        if let Some(p) = patches.get(&n) {
            let mut v = serde_json::to_value(&x).unwrap();
            merge(&mut v, p);
            x = serde_json::from_value(v).unwrap();
        }
        call.acknowledge(r, reply).unwrap();
        if restore {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
    }
    assert!(call.complete());
    json!({"events":events,"state":x})
}
fn requests(v: &Value) -> Vec<Request> {
    v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| serde_json::from_value(e["request"].clone()).unwrap())
        .collect()
}
fn checked(x: Fixture, patches: BTreeMap<usize, Value>) -> Value {
    let a = run(x.clone(), &patches, false);
    let b = run(x, &patches, true);
    assert_eq!(a, b);
    a
}

#[test]
fn view_control_preserves_full_order_and_each_saved_callback() {
    let x = fixture(u32::MAX);
    let r = checked(x.clone(), BTreeMap::new());
    let q = requests(&r);
    assert_eq!(
        &q[..4],
        &[
            Request::Invalidate(VIEW),
            Request::Buttons {
                view: VIEW,
                selector: 3
            },
            Request::Get {
                owner: ARG,
                query: Query::Mode
            },
            Request::Panel {
                panel: x.panel,
                selector: 0
            }
        ]
    );
    let positions = |query| {
        q.iter()
            .position(|r| matches!(r,Request::Get{query:q,..} if *q==query))
            .unwrap()
    };
    let order = [
        Query::Mode,
        Query::Floor,
        Query::Walls,
        Query::ZoomIn,
        Query::Status,
        Query::Title,
        Query::LotPaused,
        Query::SavedSpeedActive,
        Query::Expansion,
        Query::BuyBlocked,
        Query::LiveBlocked,
        Query::LabelsHidden,
    ];
    assert!(order.windows(2).all(|w| positions(w[0]) < positions(w[1])));
    assert_eq!(r["state"]["fields"]["blink"]["phase"], 1);
    assert_eq!(r["state"]["fields"]["blink"]["deadline"], 110);
    assert!(q.contains(&Request::Floor {
        state: ARG,
        value: 1,
        refresh: 0
    }));
    assert_eq!(
        &q[q.len() - 2..],
        &[
            Request::Visibility {
                widget: x.fields.widget(Widget::Status),
                slot: 0x98
            },
            Request::Visibility {
                widget: x.fields.widget(Widget::Label),
                slot: 0x98
            }
        ]
    );
}

#[test]
fn view_control_captures_mask_panel_and_mode_but_rereads_widget_identities() {
    let x = fixture(2);
    let mut widgets = x.fields.widgets;
    widgets[Widget::Buy as usize] = VIEW + 0xa000;
    let r = checked(
        x.clone(),
        BTreeMap::from([
            (0, json!({"dirty":0,"panel":77,"queries":{"Mode":0}})),
            (3, json!({"queries":{"Mode":4}})),
            (4, json!({"fields":{"widgets":widgets}})),
        ]),
    );
    let q = requests(&r);
    assert_eq!(
        q[3],
        Request::Panel {
            panel: x.panel,
            selector: 2
        }
    );
    assert_eq!(
        q[5],
        Request::Selected {
            widget: VIEW + 0xa000,
            value: 0
        }
    );
    assert_eq!(
        q[6],
        Request::Selected {
            widget: x.fields.widget(Widget::Build),
            value: 1
        }
    );
    assert!(q.contains(&Request::Buttons {
        view: VIEW,
        selector: 0
    }));
    assert!(!q.iter().any(|r| matches!(
        r,
        Request::Get {
            query: Query::Status,
            ..
        }
    )));
}

#[test]
fn view_control_rereads_floor_and_registered_owner_but_captures_zoom_bytes() {
    let x = fixture(0x100);
    let r = checked(
        x.clone(),
        BTreeMap::from([(
            1,
            json!({"queries":{"Floor":1,"Mode":0,"BuildFloorAvailable":128}}),
        )]),
    );
    let q = requests(&r);
    assert_eq!(
        q[1],
        Request::Selected {
            widget: x.fields.widget(Widget::FloorOne),
            value: 0
        }
    );
    assert_eq!(
        q[3],
        Request::Selected {
            widget: x.fields.widget(Widget::FloorTwo),
            value: 0
        }
    );
    assert!(!q.iter().any(|r| matches!(
        r,
        Request::Get {
            query: Query::UpperFloorAvailable,
            ..
        } | Request::Floor { .. }
    )));
    let x = fixture(0x1000);
    let r = checked(
        x.clone(),
        BTreeMap::from([
            (
                0,
                json!({"fields":{"registered":1234},"queries":{"ZoomIn":0}}),
            ),
            (2, json!({"queries":{"ZoomOut":0}})),
        ]),
    );
    let q = requests(&r);
    assert_eq!(
        q[0],
        Request::Get {
            owner: x.fields.registered,
            query: Query::ZoomIn
        }
    );
    assert_eq!(
        q[1],
        Request::Get {
            owner: 1234,
            query: Query::ZoomOut
        }
    );
    assert_eq!(
        r["state"]["enabled"][x.fields.widget(Widget::ZoomIn).to_string()],
        128
    );
    assert_eq!(
        r["state"]["enabled"][x.fields.widget(Widget::ZoomOut).to_string()],
        255
    );
}

#[test]
fn view_control_blink_keeps_string_references_and_reads_timer_after_callbacks() {
    let x = fixture(0x4000);
    let sk = key(source(0x28));
    let r = checked(
        x.clone(),
        BTreeMap::from([
            (3, json!({"texts":{sk:"later"}})),
            (4, json!({"now":u32::MAX-4,"fields":{"blink":{"first":10}}})),
        ]),
    );
    assert_eq!(
        r["state"]["texts"][key(Text {
            owner: VIEW,
            offset: 0x194
        })],
        "later"
    );
    assert_eq!(r["state"]["fields"]["blink"]["deadline"], 5);
    let mut x = fixture(0x4000);
    x.fields.blink.phase = 1;
    x.fields.blink.deadline = 100;
    let old = x.fields.widget(Widget::Status);
    let mut widgets = x.fields.widgets;
    widgets[Widget::Status as usize] = 987;
    let r = checked(
        x.clone(),
        BTreeMap::from([
            (
                4,
                json!({"fields":{"widgets":widgets,"blink":{"second":100}}}),
            ),
            (5, json!({"now":200,"fields":{"blink":{"second":30}}})),
        ]),
    );
    assert_eq!(requests(&r)[5], Request::Invalidate(old));
    assert_eq!(r["state"]["fields"]["blink"]["phase"], 2);
    assert_eq!(r["state"]["fields"]["blink"]["deadline"], 230);
}

#[test]
fn view_control_blink_reset_paths_have_distinct_after_dirty_writes() {
    let mut x = fixture(0x4000);
    x.fields.blink.phase = 2;
    x.fields.blink.deadline = 100;
    let r = checked(
        x,
        BTreeMap::from([(4, json!({"after_dirty":0xabcdef01u32}))]),
    );
    assert_eq!(r["state"]["after_dirty"], 0xabcdef01u32 & !0x4000);
    assert_eq!(r["state"]["fields"]["blink"]["deadline"], u32::MAX);
    let mut x = fixture(0x4000);
    x.fields.blink.phase = 2;
    x.texts.insert(key(source(0x28)), String::new());
    let r = checked(x, BTreeMap::new());
    assert_eq!(r["state"]["after_dirty"], 0x804000);
    assert_eq!(r["state"]["fields"]["blink"]["phase"], 0);
    let mut x = fixture(0x4000);
    x.fields.blink.phase = 99;
    x.fields.blink.deadline = 99;
    let r = checked(x, BTreeMap::new());
    assert_eq!(r["state"]["fields"]["blink"]["phase"], 99);
    assert_eq!(r["state"]["fields"]["blink"]["deadline"], u32::MAX);
}

#[test]
fn view_control_preserves_byte_predicates_and_captured_wall_speed_choices() {
    for byte in [0, 1, 2, 128, 255, 256] {
        let mut x = fixture(0x9000000);
        x.queries.insert("LiveBlocked".into(), byte);
        x.queries.insert("BuyBlocked".into(), byte);
        let r = checked(x.clone(), BTreeMap::new());
        for w in [Widget::Buy, Widget::Build] {
            assert_eq!(
                r["state"]["enabled"][x.fields.widget(w).to_string()],
                u8::from(byte as u8 == 0)
            );
        }
        for w in [Widget::Live, Widget::Camera] {
            assert_eq!(
                r["state"]["enabled"][x.fields.widget(w).to_string()],
                u8::from(byte as u8 != 1)
            );
        }
    }
    for speed in [760, 924, 994, 0, u32::MAX] {
        let mut x = fixture(0x40);
        x.queries.insert("Speed".into(), speed);
        let r = checked(
            x.clone(),
            BTreeMap::from([(4, json!({"queries":{"Speed":0}}))]),
        );
        for (w, v) in [
            (Widget::Normal, 760),
            (Widget::Fast, 924),
            (Widget::Ultra, 994),
        ] {
            assert_eq!(
                r["state"]["selected"][x.fields.widget(w).to_string()],
                i32::from(speed == v)
            );
        }
    }
    let x = fixture(0x80);
    let r = checked(
        x.clone(),
        BTreeMap::from([(6, json!({"queries":{"AvailableWalls":0}}))]),
    );
    for w in [Widget::WallsDown, Widget::WallsUp, Widget::Cutaway] {
        assert_eq!(r["state"]["enabled"][x.fields.widget(w).to_string()], 1);
    }
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
fn view_control_selection_reset_uses_fresh_registered_and_lot_owners() {
    let x = fixture(0x80000);
    let r = checked(
        x.clone(),
        BTreeMap::from([
            (0, json!({"fields":{"registered":1234},"lot":VIEW+0x3500})),
            (1, json!({"fields":{"registered":5678}})),
            (2, json!({"fields":{"registered":9012}})),
        ]),
    );
    assert_eq!(
        &requests(&r)[..4],
        &[
            Request::Speed {
                state: x.fields.registered,
                value: 760
            },
            Request::Get {
                owner: VIEW + 0x3500,
                query: Query::LotPaused
            },
            Request::Paused {
                state: 5678,
                value: 1
            },
            Request::Get {
                owner: 9012,
                query: Query::SelectionName
            },
        ]
    );
}

#[test]
fn view_control_equal_title_is_quiet_and_label_decision_survives_callback() {
    let mut x = fixture(0x2000);
    x.texts.insert(key(source(0x24)), "prior title".into());
    let r = checked(x, BTreeMap::new());
    assert_eq!(requests(&r).len(), 2);
    let x = fixture(0x8000000);
    let mut widgets = x.fields.widgets;
    widgets[Widget::Label as usize] = VIEW + 0xa000;
    let r = checked(
        x,
        BTreeMap::from([(
            4,
            json!({"fields":{"widgets":widgets},"queries":{"LabelsHidden":0}}),
        )]),
    );
    assert_eq!(
        requests(&r).last(),
        Some(&Request::Visibility {
            widget: VIEW + 0xa000,
            slot: 0x98
        })
    );
}

#[test]
fn view_control_rejects_corrupted_saved_cursors_and_wrong_reply_without_mutation() {
    let mut x = fixture(2);
    let mut state = State::constructed(760);
    state.buy.dirty = 2;
    let mut call = Update::new(VIEW, ARG);
    let input = Inputs {
        now: x.now,
        lot: x.lot,
        panel: x.panel,
    };
    for _ in 0..2 {
        let r = call
            .advance(&mut x.fields, &mut state, &mut x.enabled, input)
            .unwrap()
            .unwrap();
        call.acknowledge(r, Reply::Done).unwrap();
    }
    let r = call
        .advance(&mut x.fields, &mut state, &mut x.enabled, input)
        .unwrap()
        .unwrap();
    assert_eq!(
        r,
        Request::Get {
            owner: ARG,
            query: Query::Mode
        }
    );
    let before = call.clone();
    assert!(call.acknowledge(r, Reply::Done).is_err());
    assert_eq!(call, before);
    for patch in [json!({"mask":0}), json!({"pending":{"Get":{"owner":123}}})] {
        let mut value = serde_json::to_value(&call).unwrap();
        merge(&mut value, &patch);
        let bad: Update = serde_json::from_value(value).unwrap();
        assert!(bad.validate().is_err());
    }
    call.acknowledge(r, Reply::Number(2)).unwrap();
    let mut value = serde_json::to_value(&call).unwrap();
    value["answer"] = json!("Done");
    let mut bad: Update = serde_json::from_value(value).unwrap();
    let before = bad.clone();
    let original = (x.clone(), state.clone());
    assert!(
        bad.advance(&mut x.fields, &mut state, &mut x.enabled, input)
            .is_err()
    );
    assert_eq!(bad, before);
    assert_eq!((x, state), original);
}
#[test]
#[ignore = "private prepared inputs; Rust save/resume only, not original execution"]
fn prepared_view_control_cases_restore_every_child() {
    let v = corpus("native-view-control-inputs.json");
    assert_eq!(v["native_execution"], false);
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        let x: Fixture = serde_json::from_value(c["fixture"].clone()).unwrap();
        let p: BTreeMap<usize, Value> = serde_json::from_value(c["patches"].clone()).unwrap();
        assert_eq!(run(x.clone(), &p, false), run(x, &p, true), "case {n}");
    }
}
#[test]
#[ignore = "guarded original53ab6d caller corpus; intercepted children remain separate"]
fn view_control_matches_original_caller() {
    let v = corpus("research/sims-exe/oracle/view-control.json");
    assert_eq!(
        v["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        v["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        let x: Fixture = serde_json::from_value(c["input"]["fixture"].clone()).unwrap();
        let p: BTreeMap<usize, Value> =
            serde_json::from_value(c["input"]["patches"].clone()).unwrap();
        assert_eq!(run(x, &p, true), c["original"], "case {n}");
    }
}
