use serde_json::{Value, json};
use sims_vm::ui_owner::wait::{Condition, Observation, Poll, Reply, Request, State};
fn i(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn b(v: &Value, k: &str) -> bool {
    v[k].as_bool().unwrap()
}
fn focus(v: &Value) -> Option<i32> {
    (b(v, "focus") && i(v, "focus_image") != 0).then(|| i(v, "focus_id"))
}
fn observe(c: &Condition, v: &Value) -> Observation {
    match c {
        Condition::PanelExpanded => {
            Observation::PanelExpansion(v["tool"].as_i64().map(|n| n as i32))
        }
        Condition::Panel(_) | Condition::PanelAbsent => Observation::Panel(
            (b(v, "window")
                && b(v, "panel")
                && i(v, "panel_enabled") != 0
                && b(v, "panel_content"))
            .then(|| v["panel_name"].as_str().unwrap().into()),
        ),
        Condition::Focus(_) => Observation::Focus(focus(v)),
        Condition::Portrait(_) => Observation::Portrait {
            focus_present: b(v, "focus"),
            focus_image: i(v, "focus_image") as u32,
            resolved: (b(v, "neighbor") && b(v, "portrait_found"))
                .then(|| i(v, "portrait_image") as u32),
        },
        Condition::Control(_) => Observation::Control {
            image: (b(v, "control_found") && b(v, "control_image")).then(|| i(v, "control_id")),
            // Historical fixture label; the captured field is button+e0.
            state: i(v, "control_enabled") as u32,
        },
        Condition::Rotation => Observation::Counter(i(v, "rotation")),
        Condition::Scroll => Observation::Counter(i(v, "scroll")),
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn wait_event_poll_matches_original_services_live_reads_and_reset_order() {
    let data: Value = serde_json::from_str(include_str!("fixtures/ui-wait.json")).unwrap();
    assert_eq!(
        data["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        data["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 640);
    for row in cases {
        let c = &row["input"];
        let mut state = State {
            baseline: i(c, "baseline"),
        };
        let mut poll = Poll::begin(b(c, "owner").then_some(1), focus(&c["before"])).unwrap();
        let mut global12 = 0i16;
        let mut events = vec![];
        while let Some(request) = poll.request().cloned() {
            poll = serde_json::from_slice(&serde_json::to_vec(&poll).unwrap()).unwrap();
            let saved = (state.clone(), poll.clone());
            assert!(
                poll.acknowledge(&mut state, &Request::Got(999), Reply::Done)
                    .is_err()
            );
            assert_eq!((state.clone(), poll.clone()), saved);
            let reply = match &request {
                Request::PublishControl(id) => {
                    global12 = *id;
                    Reply::Done
                }
                Request::Query(1) => {
                    events.push(json!(["query", state.baseline]));
                    state.baseline = i(c, "query_baseline");
                    Reply::Query(
                        b(c, "query_result")
                            .then(|| [i(c, "mode") as i16, i(c, "parameter") as i16]),
                    )
                }
                Request::Observe(condition) => Reply::Observed(observe(condition, &c["observed"])),
                Request::Got(1) => {
                    events.push(json!(["got", state.baseline]));
                    state.baseline = 123456;
                    Reply::Done
                }
                _ => panic!("lost originally captured owner"),
            };
            poll.acknowledge(&mut state, &request, reply).unwrap();
        }
        assert_eq!(
            json!({"baseline":state.baseline,"global12":global12,"events":events}),
            row["original"],
            "{c}"
        );
    }
}
