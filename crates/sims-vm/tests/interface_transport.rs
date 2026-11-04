use serde_json::{Value, json};
use sims_vm::{
    interface::{
        state::State,
        transport::{Inputs, Request, SetPause, Update, UpdateRequest},
    },
    scheduler::transport::{Change, Command, Timing},
};

#[test]
fn resume_resets_timers_after_callback_without_overwriting_repause() {
    let mut paused = 256;
    let mut timing = Timing {
        last: 55,
        elapsed: 99,
    };
    let mut call = Change::new(Command::Resume);
    let request = call.advance(&mut paused, &mut timing, -1).unwrap().unwrap();
    assert_eq!(paused, 0);
    assert_eq!(
        timing,
        Timing {
            last: 55,
            elapsed: 99
        }
    );
    paused = -256;
    timing.last = 77;
    let saved = serde_json::to_vec(&call).unwrap();
    call = serde_json::from_slice(&saved).unwrap();
    assert_eq!(
        call.advance(&mut paused, &mut timing, 1).unwrap(),
        Some(request)
    );
    assert_eq!(paused, -256);
    call.acknowledge(request).unwrap();
    assert_eq!(call.advance(&mut paused, &mut timing, 1).unwrap(), None);
    assert_eq!(paused, -256);
    assert_eq!(timing, Timing::default());
    assert!(call.complete());
}

#[test]
fn pause_setter_retains_argument_across_window_callback() {
    let mut state = State::constructed(50);
    state.mode = 2;
    state.buy.dirty = 0x100;
    let mut call = SetPause::new(0x101);
    assert_eq!(
        call.advance(&mut state).unwrap(),
        Some(Request::WindowPause(0x101))
    );
    assert_eq!((state.paused, state.buy.dirty), (1, 0x800140));
    state.mode = 3;
    state.paused = 0;
    state.buy.dirty = 0;
    call.acknowledge(Request::WindowPause(0x101)).unwrap();
    assert_eq!(
        call.advance(&mut state).unwrap(),
        Some(Request::RefreshPeople)
    );
    assert_eq!(state.buy.dirty, 0);
    call.acknowledge(Request::RefreshPeople).unwrap();
    assert!(call.complete());
    // Low-byte zero: the window still receives the whole argument, no refresh.
    state.mode = 2;
    let mut call = SetPause::new(256);
    let r = call.advance(&mut state).unwrap().unwrap();
    call.acknowledge(r).unwrap();
    assert_eq!(call.advance(&mut state).unwrap(), None);
    assert_eq!(state.paused, 0);
}

#[test]
fn dirty_update_publishes_mode_before_speed_then_rereads_pause() {
    let mut state = State::constructed(100);
    state.mode = 2;
    let mut mode = 3;
    let input = Inputs {
        menu_present: false,
        menu_active: 0,
        closing: 0,
        lot_speed: 50,
    };
    let mut call = Update::new(0x800040).unwrap();
    assert_eq!(
        call.advance(&state, &mut mode, input).unwrap(),
        Some(UpdateRequest::SetSpeed(100))
    );
    assert_eq!(mode, 0);
    state.paused = 128;
    call.acknowledge(UpdateRequest::SetSpeed(100)).unwrap();
    assert_eq!(
        call.advance(&state, &mut mode, input).unwrap(),
        Some(UpdateRequest::Pause)
    );
    state.mode = 3;
    state.paused = 0;
    call.acknowledge(UpdateRequest::Pause).unwrap();
    assert_eq!(
        call.advance(&state, &mut mode, input).unwrap(),
        Some(UpdateRequest::PauseAudio)
    );
}

#[test]
fn shared_control_state_keeps_buy_and_mode_callback_writes() {
    let mut state = State::constructed(50);
    assert_eq!(state.buy.selection_aux, u32::MAX);
    assert_eq!((state.mode, state.build_tool, state.buy_tool), (3, 11, 3));
    let mut fields = state.mode_fields(1, 255);
    fields.mode = 1;
    fields.catalog = 4;
    fields.subcatalog = 0;
    assert_eq!(state.publish_mode_fields(fields), (1, 255));
    state.buy.reset_selection().unwrap();
    state.buy.dirty = 0x400;
    let fields = state.mode_fields(0, 0);
    assert_eq!(
        (fields.catalog, fields.subcatalog, fields.dirty),
        (4, 0, 0x400)
    );
    let decoded: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(state, decoded);
}

#[test]
#[ignore = "private original-instruction pause/transport corpus"]
fn pause_transport_matches_original_instructions_with_saved_callbacks() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/pause-transport.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut events = vec![];
        let result;
        if c["kind"] == "lot" {
            let mut call = Change::new(serde_json::from_value(c["command"].clone()).unwrap());
            let mut timing: Timing = serde_json::from_value(c["timing"].clone()).unwrap();
            let mut paused = c["paused"].as_i64().unwrap() as i16;
            loop {
                let Some(r) = call
                    .advance(&mut paused, &mut timing, c["gate"].as_i64().unwrap() as i32)
                    .unwrap()
                else {
                    break;
                };
                events.push(json!({"request":r,"state":{"paused":paused,"last":timing.last,"elapsed":timing.elapsed}}));
                call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                let snapshot = (paused, timing);
                assert_eq!(call.advance(&mut paused, &mut timing, 77).unwrap(), Some(r));
                assert_eq!((paused, timing), snapshot);
                let m = &c["mutations"]["Broadcast"];
                if let Some(v) = m["paused"].as_i64() {
                    paused = v as i16;
                }
                if let Some(v) = m["last"].as_u64() {
                    timing.last = v as u32;
                }
                if let Some(v) = m["elapsed"].as_u64() {
                    timing.elapsed = v as u32;
                }
                call.acknowledge(r).unwrap();
            }
            assert!(call.complete());
            result = json!({"paused":paused,"last":timing.last,"elapsed":timing.elapsed});
        } else {
            let mut state = State::constructed(c["speed"].as_i64().unwrap_or(50) as i32);
            state.mode = c["mode"].as_i64().unwrap() as i32;
            state.paused = c["paused"].as_u64().unwrap() as u8;
            state.buy.dirty = c["dirty"].as_u64().unwrap() as u32;
            let mut mode = c["lot_mode"].as_i64().unwrap_or(0) as i16;
            let observed = |s: &State, mode| {
                let mut v = json!({"mode":s.mode,"paused":s.paused,"dirty":s.buy.dirty});
                if c["kind"] == "update" {
                    v["lot_mode"] = json!(mode);
                }
                v
            };
            let patch = |s: &mut State, name: &str| {
                let m = &c["mutations"][name];
                if let Some(v) = m["mode"].as_i64() {
                    s.mode = v as i32;
                }
                if let Some(v) = m["paused"].as_u64() {
                    s.paused = v as u8;
                }
                if let Some(v) = m["dirty"].as_u64() {
                    s.buy.dirty = v as u32;
                }
            };
            if c["kind"] == "panel" {
                let mut call = SetPause::new(c["value"].as_u64().unwrap() as u32);
                while let Some(r) = call.advance(&mut state).unwrap() {
                    events.push(json!({"request":r,"state":observed(&state, mode)}));
                    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                    let before = state.clone();
                    assert_eq!(call.advance(&mut state).unwrap(), Some(r));
                    assert_eq!(before, state);
                    patch(
                        &mut state,
                        match r {
                            Request::WindowPause(_) => "WindowPause",
                            Request::RefreshPeople => "RefreshPeople",
                        },
                    );
                    call.acknowledge(r).unwrap();
                }
                assert!(call.complete());
            } else {
                let mut call = Update::new(c["mask"].as_u64().unwrap() as u32).unwrap();
                let input = Inputs {
                    menu_present: c["menu"].as_bool().unwrap(),
                    menu_active: c["active"].as_u64().unwrap() as u8,
                    closing: c["closing"].as_u64().unwrap() as u8,
                    lot_speed: c["lot_speed"].as_i64().unwrap() as i16,
                };
                while let Some(r) = call.advance(&state, &mut mode, input).unwrap() {
                    events.push(json!({"request":r,"state":observed(&state, mode)}));
                    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                    let before = mode;
                    assert_eq!(call.advance(&state, &mut mode, input).unwrap(), Some(r));
                    assert_eq!(before, mode);
                    patch(
                        &mut state,
                        match r {
                            UpdateRequest::SetSpeed(_) => "SetSpeed",
                            UpdateRequest::Pause => "Pause",
                            UpdateRequest::Resume => "Resume",
                            UpdateRequest::PauseAudio => "PauseAudio",
                            UpdateRequest::ResumeAudio => "ResumeAudio",
                        },
                    );
                    call.acknowledge(r).unwrap();
                }
                assert!(call.complete());
            }
            result = observed(&state, mode);
        }
        assert_eq!(
            json!({"events":events,"state":result}),
            row["original"],
            "case{n}: {c}"
        );
    }
}
