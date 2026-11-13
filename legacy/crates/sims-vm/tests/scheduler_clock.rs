use serde_json::{Value, json};
use sims_vm::scheduler::clock::speed_override::{
    Change as Override, Command as OverrideCommand, Notice,
};
use sims_vm::scheduler::{
    clock::{Fields, Frame, Inputs, Request, Speed, Timer, period},
    transport::Timing,
};

fn fields() -> Fields {
    Fields {
        timing: Timing::default(),
        period: 42,
        timer: Timer {
            interval: 42,
            limit: 30,
            ..Default::default()
        },
        budget_d8: 60,
        budget_dc: 90,
        poll_messages: 0,
        publish: 0,
    }
}

#[test]
fn timer_preserves_wrap_unsigned_cap_and_zero_interval() {
    let mut t = Timer {
        last: u32::MAX - 4,
        elapsed: 90,
        interval: 42,
        limit: 2,
    };
    t.update(5);
    assert_eq!(t.elapsed, 84);
    assert!(t.consume());
    assert!(t.consume());
    assert!(!t.consume());
    t.set_interval(0);
    assert!(t.consume());
    t.set_limit(-1);
    assert_eq!(t.limit, 0);
    t.last = 0;
    t.elapsed = 999;
    t.set_interval(172);
    t.update(7);
    assert_eq!(t.elapsed, 172);
    assert_eq!(t.last, 7);
    t.limit = u32::MAX;
    t.interval = u32::MAX;
    t.update(7);
    assert_eq!(t.elapsed, 1); // Native multiplication wraps before unsigned cap.
}

#[test]
fn speed_keeps_timer_history_and_callback_writes() {
    assert_eq!(
        (
            period(-1),
            period(0),
            period(500),
            period(1000),
            period(i32::MAX)
        ),
        (172, 172, 86, 1, 1)
    );
    // Exact rational form of the original f32 constant, across every input.
    for speed in 0..=1000 {
        let numerator = 345i64 * (1i64 << 32) - i64::from(speed) * 8_589_935 * 171;
        assert_eq!(period(speed), (numerator / (1i64 << 33)) as i32);
    }
    let mut f = fields();
    f.timer.last = 456;
    f.timer.elapsed = 123;
    f.timing = Timing {
        last: 99,
        elapsed: 333,
    };
    let mut call = Speed::new(1001);
    let (mut speed, mut saved) = (50, -1);
    assert!(call.advance(&mut f, &mut speed, &mut saved).unwrap());
    assert_eq!((speed, saved), (1000, -1));
    assert_eq!(f.timing, Timing::default());
    assert_eq!(
        f.timer,
        Timer {
            last: 456,
            elapsed: 123,
            interval: 1,
            limit: 30
        }
    );
    let before = f.clone();
    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
    assert!(call.advance(&mut f, &mut speed, &mut saved).unwrap());
    assert_eq!(before, f);
    saved = 0;
    f.timing.last = 77;
    call.acknowledge().unwrap();
    assert!(!call.advance(&mut f, &mut speed, &mut saved).unwrap());
    assert_eq!(saved, 1000);
    assert_eq!(f.timing.last, 77);
    assert!(call.complete());
}

#[test]
fn elapsed_clock_reloads_time_after_callbacks_and_clears_publication_after_return() {
    let mut f = fields();
    let mut input = Inputs {
        now: 100,
        stopping: 0,
        step_word: 0,
    };
    let mut call = Frame::default();
    assert_eq!(
        call.advance(&mut f, &mut input).unwrap(),
        Some(Request::ApplicationState)
    );
    call.acknowledge(Request::ApplicationState, 256).unwrap(); // Low byte !=1.
    assert_eq!(
        call.advance(&mut f, &mut input).unwrap(),
        Some(Request::UpdateLot)
    );
    assert_eq!(
        f.timing,
        Timing {
            last: 100,
            elapsed: 0
        }
    );
    assert_eq!(f.timer.elapsed, 0);
    call.acknowledge(Request::UpdateLot, 0).unwrap();
    assert_eq!(
        call.advance(&mut f, &mut input).unwrap(),
        Some(Request::ProcessRequests)
    );
    f.publish = 1;
    call.acknowledge(Request::ProcessRequests, 0).unwrap();
    assert_eq!(
        call.advance(&mut f, &mut input).unwrap(),
        Some(Request::Publish)
    );
    f.publish = 2;
    input.now = 107;
    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
    call.acknowledge(Request::Publish, 0).unwrap();
    assert_eq!(call.advance(&mut f, &mut input).unwrap(), None);
    assert_eq!(f.publish, 0);
    assert_eq!((f.timer.last, f.timer.elapsed), (107, 7));
    assert_eq!(
        f.timing,
        Timing {
            last: 100,
            elapsed: 0
        }
    );
    assert!(call.complete());
}

#[test]
fn elapsed_clock_preserves_stop_step_and_message_poll_precedence() {
    for (trap, stop, step, expected) in [(0, 1, 256, 256), (0, 0, -32768, 0), (1, 1, 256, 256)] {
        let mut f = fields();
        f.poll_messages = trap;
        let mut input = Inputs {
            now: 100,
            stopping: stop,
            step_word: step,
        };
        let mut call = Frame::default();
        let mut requests = vec![];
        while let Some(r) = call.advance(&mut f, &mut input).unwrap() {
            requests.push(r);
            call.acknowledge(r, 0).unwrap();
        }
        assert_eq!(input.step_word, expected);
        assert_eq!(
            requests,
            if trap != 0 {
                vec![
                    Request::ApplicationState,
                    Request::UpdateLot,
                    Request::PeekMessage,
                ]
            } else {
                vec![Request::ApplicationState, Request::UpdateLot]
            }
        );
        assert!(!call.trapped());
    }
}

fn merge(value: &mut Value, mutation: &Value) {
    if let Some(items) = mutation.as_object() {
        for (key, m) in items {
            if m.is_object() {
                merge(&mut value[key], m);
            } else {
                value[key] = m.clone();
            }
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn gameplay_clock_matches_original_message_poll_and_saved_callbacks() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/scheduler-clock-gameplay.json")).unwrap();
    assert_eq!(
        report["capture_hashes"]["capture-000000ec-00010000.bin"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    assert_eq!(report["original_execution"], true);
    assert_eq!(report["supplied_instructions"], false);
    assert_eq!(report["peek_message_calls"], 89);
    assert_eq!(report["cases"].as_array().unwrap().len(), 239);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut f: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
        let mut inputs: Inputs = serde_json::from_value(c["inputs"].clone()).unwrap();
        let mut call = Frame::default();
        let mut events = vec![];
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        while let Some(request) = call.advance(&mut f, &mut inputs).unwrap() {
            assert!(events.len() < 1000, "unbounded recovered clock case{n}");
            let mut state = json!({"fields":f,"inputs":inputs});
            events.push(json!({"request":request,"state":state}));
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            assert_eq!(call.advance(&mut f, &mut inputs).unwrap(), Some(request));
            assert_eq!(state, json!({"fields":f,"inputs":inputs}));
            let key = serde_json::to_value(request)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            let count = counts.entry(key.clone()).or_default();
            let action = &c["actions"][&key][*count];
            *count += 1;
            merge(&mut state, &action["mutation"]);
            f = serde_json::from_value(state["fields"].clone()).unwrap();
            inputs = serde_json::from_value(state["inputs"].clone()).unwrap();
            call.acknowledge(request, action["returned"].as_u64().unwrap_or(0) as u32)
                .unwrap();
        }
        assert!(call.complete());
        assert_eq!(
            json!({"events":events,"state":{"fields":f,"inputs":inputs},"trapped":false}),
            row["original"],
            "case{n}: {c}"
        );
    }
    // Do not reinterpret an old checkpoint which stopped on protection bytes.
    let old: Frame =
        serde_json::from_value(json!({"stage":"Complete","pending":"DebugBreak","trapped":true}))
            .unwrap();
    assert!(
        old.validate()
            .unwrap_err()
            .contains("obsolete protected clock")
    );
}

#[test]
fn speed_override_preserves_captured_speed_and_nested_notice_order() {
    for command in [OverrideCommand::Boost(777), OverrideCommand::Restore] {
        let mut f = fields();
        let (mut speed, mut saved) = (
            50,
            if command == OverrideCommand::Restore {
                100
            } else {
                -1
            },
        );
        let mut call = Override::new(command);
        let first = call
            .advance(&mut f, &mut speed, &mut saved)
            .unwrap()
            .unwrap();
        assert_eq!(
            first,
            Notice {
                event: 0xc2,
                argument: 0
            }
        );
        speed = 999;
        saved = 0;
        f.timing.last = 77;
        call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        assert_eq!(
            call.advance(&mut f, &mut speed, &mut saved).unwrap(),
            Some(first)
        );
        assert_eq!((speed, saved, f.timing.last), (999, 0, 77));
        call.acknowledge(first).unwrap();
        let second = call
            .advance(&mut f, &mut speed, &mut saved)
            .unwrap()
            .unwrap();
        assert_eq!(
            second,
            Notice {
                event: 0x109,
                argument: u32::from(command != OverrideCommand::Restore)
            }
        );
        assert_eq!(
            saved,
            if command == OverrideCommand::Restore {
                -1
            } else {
                50
            }
        );
        assert_eq!((speed, f.timing.last), (999, 77));
        saved = 12;
        call.acknowledge(second).unwrap();
        assert_eq!(call.advance(&mut f, &mut speed, &mut saved).unwrap(), None);
        assert_eq!(saved, 12);
        assert!(call.complete());
    }
}

#[test]
fn speed_override_gates_leave_timers_and_speed_untouched() {
    for (command, mut saved) in [
        (OverrideCommand::Restore, -1),
        (OverrideCommand::Boost(1000), 0),
    ] {
        let mut f = fields();
        f.timing = Timing {
            last: 99,
            elapsed: 400,
        };
        let before = f.clone();
        let mut speed = 50;
        let mut call = Override::new(command);
        assert_eq!(call.advance(&mut f, &mut speed, &mut saved).unwrap(), None);
        assert_eq!(f, before);
        assert_eq!(speed, 50);
        assert!(call.complete());
    }
}

#[test]
#[ignore = "private original-instruction timer/speed/elapsed-clock corpus"]
fn scheduler_clock_matches_original_instructions_with_saved_callbacks() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/scheduler-clock.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        // These archival placeholders were not executable gameplay semantics.
        // The playable-image corpus below supersedes this branch entirely.
        if row["original"]["trapped"] == true {
            continue;
        }
        let result = if c["kind"] == "timer" {
            let mut timer: Timer = serde_json::from_value(c["timer"].clone()).unwrap();
            timer.update(c["now"].as_u64().unwrap() as u32);
            let after = timer;
            let taken = timer.consume();
            json!({"after_update":after,"taken":taken,"timer":timer})
        } else if c["kind"] == "speed" {
            let mut f: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
            let mut speed = c["speed"].as_i64().unwrap() as i16;
            let mut saved = c["saved_speed"].as_i64().unwrap() as i16;
            let mut call = Speed::new(c["argument"].as_i64().unwrap() as i32);
            let mut events = vec![];
            while call.advance(&mut f, &mut speed, &mut saved).unwrap() {
                let mut state = json!({"fields":f,"speed":speed,"saved_speed":saved});
                events.push(json!({"request":"Broadcast","state":state}));
                call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                assert!(call.advance(&mut f, &mut speed, &mut saved).unwrap());
                assert_eq!(state, json!({"fields":f,"speed":speed,"saved_speed":saved}));
                merge(&mut state, &c["mutation"]);
                f = serde_json::from_value(state["fields"].clone()).unwrap();
                speed = state["speed"].as_i64().unwrap() as i16;
                saved = state["saved_speed"].as_i64().unwrap() as i16;
                call.acknowledge().unwrap();
            }
            assert!(call.complete());
            json!({"events":events,"state":{"fields":f,"speed":speed,"saved_speed":saved}})
        } else if c["kind"] == "override" {
            let mut f: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
            let mut speed = c["speed"].as_i64().unwrap() as i16;
            let mut saved = c["saved_speed"].as_i64().unwrap() as i16;
            let mut call = Override::new(serde_json::from_value(c["command"].clone()).unwrap());
            let mut events = vec![];
            while let Some(request) = call.advance(&mut f, &mut speed, &mut saved).unwrap() {
                let mut state = json!({"fields":f,"speed":speed,"saved_speed":saved});
                events.push(json!({"request":request,"state":state}));
                call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                assert_eq!(
                    call.advance(&mut f, &mut speed, &mut saved).unwrap(),
                    Some(request)
                );
                assert_eq!(state, json!({"fields":f,"speed":speed,"saved_speed":saved}));
                merge(&mut state, &c["mutations"][request.event.to_string()]);
                f = serde_json::from_value(state["fields"].clone()).unwrap();
                speed = state["speed"].as_i64().unwrap() as i16;
                saved = state["saved_speed"].as_i64().unwrap() as i16;
                call.acknowledge(request).unwrap();
            }
            assert!(call.complete());
            json!({"events":events,"state":{"fields":f,"speed":speed,"saved_speed":saved}})
        } else {
            let mut f: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
            let mut inputs: Inputs = serde_json::from_value(c["inputs"].clone()).unwrap();
            let mut call = Frame::default();
            let mut events = vec![];
            let mut counts = std::collections::BTreeMap::<String, usize>::new();
            while let Some(request) = call.advance(&mut f, &mut inputs).unwrap() {
                assert!(events.len() < 1000, "unbounded clock case{n}");
                let mut state = json!({"fields":f,"inputs":inputs});
                events.push(json!({"request":request,"state":state}));
                call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                assert_eq!(call.advance(&mut f, &mut inputs).unwrap(), Some(request));
                assert_eq!(state, json!({"fields":f,"inputs":inputs}));
                let key = serde_json::to_value(request)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned();
                let count = counts.entry(key.clone()).or_default();
                let action = &c["actions"][&key][*count];
                *count += 1;
                merge(&mut state, &action["mutation"]);
                f = serde_json::from_value(state["fields"].clone()).unwrap();
                inputs = serde_json::from_value(state["inputs"].clone()).unwrap();
                call.acknowledge(request, action["returned"].as_u64().unwrap_or(0) as u32)
                    .unwrap();
            }
            assert!(call.complete());
            json!({"events":events,"state":{"fields":f,"inputs":inputs},"trapped":call.trapped()})
        };
        assert_eq!(result, row["original"], "case{n}: {c}");
    }
}

#[test]
fn normal_speed_predicate_uses_ordered_live_fields_and_rejects_unordered_mood() {
    let mut world = sims_vm::World::default();
    world.person = Some(sims_vm::person::WorldState {
        person_order: vec![1, 2],
        ..Default::default()
    });
    // The second missing entry must never be read after the first qualifies.
    let mut object = sims_vm::Object::default();
    object.data.insert(34, 0);
    object.person = Some([(65, -32768), (32, 0)].into_iter().collect());
    object.motives = Some([0; 16]);
    world.objects.insert(1, object);
    assert!(sims_vm::scheduler::clock::needs_normal_speed(&world).unwrap());
    world.person.as_mut().unwrap().person_order = vec![1];
    for bits in [0, 0x80000000, 1f32.to_bits(), f32::INFINITY.to_bits()] {
        world.objects.get_mut(&1).unwrap().motives.as_mut().unwrap()[11] = bits;
        assert!(sims_vm::scheduler::clock::needs_normal_speed(&world).unwrap());
    }
    for bits in [
        (-1f32).to_bits(),
        f32::NEG_INFINITY.to_bits(),
        0x7fc01234,
        0xffc01234,
    ] {
        world.objects.get_mut(&1).unwrap().motives.as_mut().unwrap()[11] = bits;
        assert!(!sims_vm::scheduler::clock::needs_normal_speed(&world).unwrap());
    }
    // Each gate permits the next field to remain genuinely absent.
    let object = world.objects.get_mut(&1).unwrap();
    object.person.as_mut().unwrap().remove(&32);
    assert!(!sims_vm::scheduler::clock::needs_normal_speed(&world).unwrap());
    let object = world.objects.get_mut(&1).unwrap();
    object.person.as_mut().unwrap().insert(65, 2);
    object.motives = None;
    assert!(!sims_vm::scheduler::clock::needs_normal_speed(&world).unwrap());
    let object = world.objects.get_mut(&1).unwrap();
    object.data.insert(34, 256);
    object.person = None;
    assert!(!sims_vm::scheduler::clock::needs_normal_speed(&world).unwrap());
}

#[test]
#[ignore = "private guarded original442f9b/45edc5 comparison corpus"]
fn clock_person_predicate_matches_original_fields_and_float_edges() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/clock-predicate.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 255);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let mut world = sims_vm::World::default();
        world.person = Some(Default::default());
        for (i, p) in row["people"].as_array().unwrap().iter().enumerate() {
            let id = i as i16 + 1;
            let mut object = sims_vm::Object::default();
            object.data.insert(34, p["hidden"].as_i64().unwrap() as i16);
            object.person = Some(
                [
                    (65, p["mode"].as_i64().unwrap() as i16),
                    (32, p["state"].as_i64().unwrap() as i16),
                ]
                .into_iter()
                .collect(),
            );
            let mut motives = [0; 16];
            motives[11] = p["mood"].as_u64().unwrap() as u32;
            object.motives = Some(motives);
            world.objects.insert(id, object);
            world.person.as_mut().unwrap().person_order.push(id);
        }
        assert_eq!(
            sims_vm::scheduler::clock::needs_normal_speed(&world).unwrap(),
            row["original"].as_bool().unwrap(),
            "case{n}"
        );
    }
}
