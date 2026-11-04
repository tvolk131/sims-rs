use sims_vm::{
    interface::{
        publication::{Fields, Frame, Inputs, Request},
        speed::Timers,
        state::State,
    },
    scheduler::clock::Timer,
};

fn input(now: u32) -> Inputs {
    Inputs {
        now,
        panel_owner: Some(10),
    }
}
fn state() -> State {
    let mut s = State::constructed(760);
    s.buy.dirty = 0;
    s.publication = Some(Fields {
        after_dirty: 0x400,
        last: 90,
    });
    s.speed_timers = Some(Timers {
        first: Timer {
            elapsed: 9,
            interval: 10,
            last: 100,
            limit: 0,
        },
        second: Timer {
            elapsed: 28,
            interval: 30,
            last: 100,
            limit: 0,
        },
    });
    s
}

#[test]
fn publication_polls_shared_timers_once_and_retains_both_callback_returns() {
    let mut s = state();
    let mut f = Frame::new(0);
    let world = Request::World(0x6078001);
    assert_eq!(f.advance(&mut s, input(102)).unwrap(), Some(world));
    assert_eq!(s.speed_timers.unwrap().first.elapsed, 1);
    assert_eq!(s.speed_timers.unwrap().second.elapsed, 0);
    assert_eq!(s.publication.unwrap().last, 90);
    let checkpoint = serde_json::to_vec(&(s.clone(), f.clone())).unwrap();
    for _ in 0..2 {
        (s, f) = serde_json::from_slice(&checkpoint).unwrap();
        let before = s.clone();
        assert_eq!(f.advance(&mut s, input(900)).unwrap(), Some(world));
        assert_eq!(s, before); // A work budget is not a new frame/timer poll.
        assert!(f.acknowledge(Request::World(1)).is_err());
        assert_eq!(f.request(), Some(world));
        // The original world callback can change all these shared values.
        s.buy.dirty = 0x800;
        s.publication.as_mut().unwrap().after_dirty = 0x100;
        s.speed_timers.as_mut().unwrap().first.elapsed = 777;
        f.acknowledge(world).unwrap();
        let panels = Request::Panels(Some(20));
        assert_eq!(
            f.advance(
                &mut s,
                Inputs {
                    now: 901,
                    panel_owner: Some(20)
                }
            )
            .unwrap(),
            Some(panels)
        );
        assert_eq!(s.buy.dirty, 0x800);
        let saved = serde_json::to_vec(&(s.clone(), f.clone())).unwrap();
        (s, f) = serde_json::from_slice(&saved).unwrap();
        let before = s.clone();
        assert_eq!(f.advance(&mut s, input(999)).unwrap(), Some(panels));
        assert_eq!(s, before);
        s.buy.dirty = 0x200;
        s.publication.as_mut().unwrap().after_dirty = 0x80;
        f.acknowledge(panels).unwrap();
        assert!(!f.complete());
        assert_eq!(f.advance(&mut s, input(1000)).unwrap(), None);
        assert!(f.complete());
        assert_eq!(s.buy.dirty, 0x80); // Replacement, not OR, and not entry state.
        assert_eq!(s.publication.unwrap().last, 1000);
        assert_eq!(s.speed_timers.unwrap().first.elapsed, 777);
        let done = s.clone();
        assert_eq!(f.advance(&mut s, input(2000)).unwrap(), None);
        assert_eq!(s, done);
    }
}

#[test]
fn publication_force_uses_low_byte_and_does_not_consume_timer_credit() {
    for (force, expected, credit) in [
        (0, 0x6078001, [1, 0]),
        (256, 0x6078001, [1, 0]),
        (1, u32::MAX, [11, 30]),
        (0xffffffff, u32::MAX, [11, 30]),
    ] {
        let mut s = state();
        let mut f = Frame::new(force);
        assert_eq!(
            f.advance(&mut s, input(102)).unwrap(),
            Some(Request::World(expected))
        );
        let timers = s.speed_timers.unwrap();
        assert_eq!([timers.first.elapsed, timers.second.elapsed], credit);
        assert_eq!([timers.first.last, timers.second.last], [102, 102]);
    }
}

#[test]
fn publication_empty_and_absent_panel_paths_leave_final_words_unwritten() {
    let mut s = state();
    let mut f = Frame::new(0);
    assert_eq!(f.advance(&mut s, input(100)).unwrap(), None);
    assert!(f.complete());
    assert_eq!(
        s.publication.unwrap(),
        Fields {
            after_dirty: 0x400,
            last: 90
        }
    );
    assert_eq!(s.buy.dirty, 0);
    s.speed_timers = None;
    s.publication = None;
    let before = s.clone();
    let mut f = Frame::new(1);
    assert_eq!(
        f.advance(
            &mut s,
            Inputs {
                now: 200,
                panel_owner: None
            }
        )
        .unwrap(),
        None
    );
    assert!(f.complete());
    assert_eq!(s, before);
    // Clearing the receiver *after* world publication does not repeat that gate.
    let mut s = state();
    let mut f = Frame::new(0);
    let request = f.advance(&mut s, input(102)).unwrap().unwrap();
    f.acknowledge(request).unwrap();
    assert_eq!(
        f.advance(
            &mut s,
            Inputs {
                now: 103,
                panel_owner: None
            }
        )
        .unwrap(),
        Some(Request::Panels(None))
    );
    assert!(!f.complete());
}

#[test]
fn publication_preserves_timer_wrap_first_poll_and_one_consume_per_frame() {
    let mut s = state();
    s.speed_timers = Some(Timers {
        first: Timer {
            elapsed: 90,
            interval: 10,
            last: u32::MAX - 1,
            limit: 0,
        },
        second: Timer {
            elapsed: 999,
            interval: 30,
            last: 0,
            limit: 0,
        },
    });
    let mut f = Frame::new(0);
    assert_eq!(
        f.advance(&mut s, input(1)).unwrap(),
        Some(Request::World(0x6078001))
    );
    assert_eq!(s.speed_timers.unwrap().first.elapsed, 83);
    assert_eq!(s.speed_timers.unwrap().second.elapsed, 0);
    // Timers with no due interval publish independently, including zero interval.
    s.speed_timers = Some(Timers {
        first: Timer {
            elapsed: 0,
            interval: 0,
            last: 1,
            limit: 0,
        },
        second: Timer {
            elapsed: 0,
            interval: 30,
            last: 1,
            limit: 0,
        },
    });
    s.buy.dirty = 0;
    let mut f = Frame::new(0);
    assert_eq!(
        f.advance(&mut s, input(1)).unwrap(),
        Some(Request::World(1))
    );
}

#[test]
fn historical_publication_fields_are_not_invented_or_partially_mutated() {
    let mut value = serde_json::to_value(state()).unwrap();
    value.as_object_mut().unwrap().remove("publication");
    let mut s: State = serde_json::from_value(value.clone()).unwrap();
    let mut f = Frame::new(1);
    let before = f.clone();
    assert!(f.advance(&mut s, input(200)).is_err());
    assert_eq!(f, before);
    assert_eq!(serde_json::to_value(&s).unwrap(), value);
    s.publication = Some(Fields::default());
    s.speed_timers = None;
    let before = s.clone();
    assert!(f.advance(&mut s, input(200)).is_err());
    assert_eq!(s, before);
}

#[test]
#[ignore = "private guarded4ce390 caller/timer corpus; child bodies are separate"]
fn publication_frame_matches_original_caller_instructions() {
    use serde_json::{Value, json};
    fn snapshot(s: &State, input: Inputs) -> Value {
        json!({"dirty":s.buy.dirty,"publication":s.publication,"timers":s.speed_timers,
               "panel_owner":input.panel_owner,"now":input.now})
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/interface-publication-frame.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 652);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut s = State::constructed(760);
        s.buy.dirty = c["dirty"].as_u64().unwrap() as u32;
        s.publication = Some(serde_json::from_value(c["publication"].clone()).unwrap());
        s.speed_timers = Some(serde_json::from_value(c["timers"].clone()).unwrap());
        let mut input = Inputs {
            now: c["now"].as_u64().unwrap() as u32,
            panel_owner: c["panel_owner"].as_u64(),
        };
        let mut f = Frame::new(c["force"].as_u64().unwrap() as u32);
        let mut events = vec![];
        for _ in 0..4 {
            let Some(request) = f.advance(&mut s, input).unwrap() else {
                break;
            };
            events.push(json!({"request":request,"state":snapshot(&s,input)}));
            let bytes = serde_json::to_vec(&(s.clone(), f.clone())).unwrap();
            (s, f) = serde_json::from_slice(&bytes).unwrap();
            let before = s.clone();
            assert_eq!(f.advance(&mut s, input).unwrap(), Some(request));
            assert_eq!(s, before);
            let name = match request {
                Request::World(_) => "World",
                Request::Panels(_) => "Panels",
            };
            if let Some(mutations) = c["mutations"][name].as_object() {
                for (key, value) in mutations {
                    match key.as_str() {
                        "dirty" => s.buy.dirty = value.as_u64().unwrap() as u32,
                        "after_dirty" => {
                            s.publication.as_mut().unwrap().after_dirty =
                                value.as_u64().unwrap() as u32
                        }
                        "last" => {
                            s.publication.as_mut().unwrap().last = value.as_u64().unwrap() as u32
                        }
                        "first_elapsed" => {
                            s.speed_timers.as_mut().unwrap().first.elapsed =
                                value.as_u64().unwrap() as u32
                        }
                        "now" => input.now = value.as_u64().unwrap() as u32,
                        "panel_owner" => input.panel_owner = value.as_u64(),
                        _ => panic!("unknown native mutation {key}"),
                    }
                }
            }
            f.acknowledge(request).unwrap();
        }
        assert!(f.complete());
        assert_eq!(
            json!({"events":events,"state":snapshot(&s,input)}),
            row["original"],
            "{c}"
        );
    }
}
