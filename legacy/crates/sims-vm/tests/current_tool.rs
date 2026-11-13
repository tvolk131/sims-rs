use serde_json::{Value, json};
use sims_vm::interface::current_tool::{Change, Listeners, Request};

fn saved<T: serde::Serialize + serde::de::DeserializeOwned>(v: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(v).unwrap()).unwrap()
}
fn listeners() -> Listeners {
    let mut l = Listeners::default();
    for id in [3, 2, 1] {
        l.construct(id).unwrap();
        l.register(id).unwrap();
    }
    l
}

#[test]
fn current_tool_same_owner_still_resets_enters_and_notifies_in_order() {
    let mut current = Some(11);
    let mut c = Change::new(Some(11)).unwrap();
    let mut l = listeners();
    let mut events = vec![];
    while let Some(r) = c.advance(&mut current, &l).unwrap() {
        c = saved(&c);
        assert_eq!(c.advance(&mut current, &l).unwrap(), Some(r));
        events.push((r, current));
        let before = c.clone();
        assert!(c.acknowledge(Request::Reset(99)).is_err());
        assert_eq!(c, before);
        match r {
            Request::Reset(_) => current = Some(22), //publication overwrites this
            Request::Enter(_) => current = Some(33), //publication must preserve this
            Request::Old { listener: 1, .. } => l.next.insert(1, Some(3)).map(|_| ()).unwrap(),
            _ => (),
        }
        c.acknowledge(r).unwrap();
    }
    assert_eq!(
        events,
        vec![
            (Request::Reset(11), Some(11)),
            (Request::Enter(11), Some(11)),
            (
                Request::Old {
                    listener: 1,
                    tool: Some(11)
                },
                Some(33)
            ),
            (
                Request::New {
                    listener: 1,
                    tool: Some(11)
                },
                Some(33)
            ),
            (
                Request::Old {
                    listener: 3,
                    tool: Some(11)
                },
                Some(33)
            ),
            (
                Request::New {
                    listener: 3,
                    tool: Some(11)
                },
                Some(33)
            ),
        ]
    );
    assert!(c.complete());
    assert_eq!(current, Some(33));
}

#[test]
fn listener_head_is_read_after_enter_and_self_unlink_keeps_second_call_but_stops_walk() {
    let mut current = None;
    let mut l = listeners();
    let mut c = Change::new(Some(22)).unwrap();
    assert_eq!(
        c.advance(&mut current, &l).unwrap(),
        Some(Request::Enter(22))
    );
    l.head = Some(2);
    c.acknowledge(Request::Enter(22)).unwrap();
    let old = Request::Old {
        listener: 2,
        tool: None,
    };
    assert_eq!(c.advance(&mut current, &l).unwrap(), Some(old));
    l.unlink(2).unwrap();
    assert_eq!(l.head, Some(3));
    (c, l) = saved(&(c, l));
    c.acknowledge(old).unwrap();
    let new = Request::New {
        listener: 2,
        tool: Some(22),
    };
    assert_eq!(c.advance(&mut current, &l).unwrap(), Some(new));
    c.acknowledge(new).unwrap();
    assert_eq!(c.advance(&mut current, &l).unwrap(), None);
    assert!(c.complete());
}

#[test]
fn null_selection_and_duplicate_registration_preserve_native_distinctions() {
    let mut l = listeners();
    let before = l.clone();
    l.register(2).unwrap();
    assert_eq!(l, before);
    l.unlink(2).unwrap();
    assert_eq!(l.next[&1], Some(3));
    assert_eq!(l.next[&2], None);
    l.register(2).unwrap();
    assert_eq!(l.head, Some(2));
    let mut current = Some(11);
    let mut c = Change::new(None).unwrap();
    assert_eq!(
        c.advance(&mut current, &l).unwrap(),
        Some(Request::Reset(11))
    );
    c.acknowledge(Request::Reset(11)).unwrap();
    assert_eq!(
        c.advance(&mut current, &l).unwrap(),
        Some(Request::Old {
            listener: 2,
            tool: Some(11)
        })
    );
    assert_eq!(current, None);
    assert!(Change::new(Some(0)).is_err());
}

#[test]
#[ignore = "requires private guarded current-tool original execution"]
fn current_tool_matches_recovered_instructions_with_callback_mutations() {
    let report: Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var("SIMS_CURRENT_TOOL_ORACLE").expect("explicit native output required"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], true);
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 216);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut current: Option<u32> = serde_json::from_value(input["current"].clone()).unwrap();
        let mut l: Listeners = serde_json::from_value(input["listeners"].clone()).unwrap();
        let mut c = Change::new(serde_json::from_value(input["target"].clone()).unwrap()).unwrap();
        let mut events = vec![];
        while let Some(r) = c.advance(&mut current, &l).unwrap() {
            c = saved(&c);
            let event = events.len();
            events.push(json!({"request":r,"current":current,"listeners":l}));
            for patch in input["changes"].as_array().unwrap() {
                if patch["event"].as_u64().unwrap() as usize != event {
                    continue;
                }
                if patch.get("current").is_some() {
                    current = serde_json::from_value(patch["current"].clone()).unwrap();
                }
                if patch.get("head").is_some() {
                    l.head = serde_json::from_value(patch["head"].clone()).unwrap();
                }
                if let Some(next) = patch["next"].as_object() {
                    for (key, value) in next {
                        l.next.insert(
                            key.parse().unwrap(),
                            serde_json::from_value(value.clone()).unwrap(),
                        );
                    }
                }
            }
            c.acknowledge(r).unwrap();
        }
        assert!(c.complete());
        assert_eq!(
            json!({"events":events,"current":current,"listeners":l}),
            row["original"],
            "case {n}"
        );
    }
}
