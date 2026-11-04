use serde_json::Value;
use sims_sim::lot_presenter::motion::State;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_centering_uses_native_motion_curves_and_samples() {
    use sims_sim::lot_presenter::centering::{self, Cursor, Reply, Request};
    let report: Value =
        serde_json::from_str(include_str!("fixtures/camera-center-motion.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 160);
    let sync = |state: &centering::State, motion: &mut State| {
        motion.active = state.motion_active;
        motion.policy = state.motion_policy;
        motion.duration = state.motion_duration;
    };
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: centering::State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut motion: State = serde_json::from_value(c["motion"].clone()).unwrap();
        let mut cursor = Cursor::begin(
            &state,
            serde_json::from_value(c["point"].clone()).unwrap(),
            c["target_level"].as_i64().unwrap() as i32,
            c["style"].as_u64().unwrap() as u8,
            c["immediate"].as_u64().unwrap() as u8,
        );
        let mut events = Vec::new();
        while let Some(request) = cursor.request() {
            sync(&state, &mut motion);
            (state, motion, cursor) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, &motion, &cursor)).unwrap())
                    .unwrap();
            assert_eq!(cursor.request(), Some(request.clone()));
            let index = events.len();
            events.push(serde_json::json!({"request":request,"state":state,"motion":motion}));
            if let Request::ChangeFloor { level, .. } = request {
                state.level = level;
            }
            for change in c["changes"].as_array().unwrap() {
                if change["event"].as_u64().unwrap() as usize != index {
                    continue;
                }
                let mut patched = serde_json::to_value(&state).unwrap();
                for (key, value) in change["state"].as_object().unwrap() {
                    patched[key] = value.clone();
                }
                state = serde_json::from_value(patched).unwrap();
            }
            let reply = match request {
                Request::Project { .. } => {
                    Reply::Screen(serde_json::from_value(c["screen"].clone()).unwrap())
                }
                Request::ConfigureMotion { .. } | Request::SampleMotion { .. } => motion
                    .center_service(&mut state, &request, c["now"].as_u64().unwrap() as u32)
                    .unwrap(),
                _ => Reply::Done,
            };
            cursor.acknowledge(&mut state, &request, reply).unwrap();
        }
        assert!(cursor.complete());
        sync(&state, &mut motion);
        assert_eq!(
            serde_json::json!({"events":events,"state":state,"motion":motion}),
            row["original"],
            "combined motion {n}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_camera_cancellation_flags_float_restore_and_callbacks() {
    use sims_sim::lot_presenter::{
        centering,
        motion::cancel::{Cursor, Reply, Request},
    };
    let report: Value = serde_json::from_str(include_str!("fixtures/camera-cancel.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 256);
    let mut documented_snan_discrepancies = 0;
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: centering::State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut cursor = Cursor::begin(&mut state);
        let mut events = Vec::new();
        while let Some(request) = cursor.request().cloned() {
            (state, cursor) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, &cursor)).unwrap()).unwrap();
            assert_eq!(cursor.request(), Some(&request));
            let wrong = if request == Request::ResolveSurface {
                Reply::Done
            } else {
                Reply::Surface(7)
            };
            let before = cursor.clone();
            assert!(cursor.acknowledge(&request, wrong).is_err());
            assert_eq!(cursor, before);
            let index = events.len();
            events.push(serde_json::json!({"request":request,"state":state}));
            for change in c["changes"].as_array().unwrap() {
                if change["event"].as_u64().unwrap() as usize != index {
                    continue;
                }
                let mut patched = serde_json::to_value(&state).unwrap();
                for (key, value) in change["state"].as_object().unwrap() {
                    patched[key] = value.clone();
                }
                state = serde_json::from_value(patched).unwrap();
            }
            let reply = if request == Request::ResolveSurface {
                Reply::Surface(7)
            } else {
                Reply::Done
            };
            cursor.acknowledge(&request, reply).unwrap();
            assert!(cursor.acknowledge(&request, Reply::Done).is_err());
        }
        assert!(cursor.complete());
        let mut expected = row["original"].clone();
        let saved = c["state"]["saved_value_bits"].as_u64().unwrap() as u32;
        let signaling = saved & 0x7fc00000 == 0x7f800000 && saved & 0x3fffff != 0;
        if signaling && !events.is_empty() {
            // Unicorn2.1.4 preserves SNaN in this FLD/FSTP pair. Keep the raw
            // oracle intact; Intel SDM vol1 §4.8.3.5/table4-7 and vol2 FLD
            // require quieting under masked invalid exceptions. Only this
            // copied field differs; every callback and other word still compare.
            assert_eq!(
                expected["events"][0]["state"]["application_value_bits"],
                saved
            );
            documented_snan_discrepancies += 1;
            let quiet_copy = |s: &mut Value| {
                if s["application_value_bits"] == saved {
                    s["application_value_bits"] = Value::from(saved | 0x400000);
                }
            };
            for event in expected["events"].as_array_mut().unwrap() {
                quiet_copy(&mut event["state"]);
            }
            quiet_copy(&mut expected["state"]);
        }
        assert_eq!(
            serde_json::json!({"events":events,"state":state}),
            expected,
            "cancel {n}"
        );
    }
    assert_eq!(documented_snan_discrepancies, 16);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_camera_motion_curves_clock_wrap_and_float_conversion() {
    let report: Value = serde_json::from_str(include_str!("fixtures/camera-motion.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 576);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        if c["configure"].as_bool().unwrap() {
            state.configure(
                serde_json::from_value(c["start"].clone()).unwrap(),
                serde_json::from_value(c["target"].clone()).unwrap(),
                c["style"].as_u64().unwrap() as u32,
                c["now"].as_u64().unwrap() as u32,
            );
        }
        let initial = state.clone();
        assert_eq!(
            serde_json::to_value(&initial).unwrap(),
            row["original"]["initial"],
            "motion initial {n}"
        );
        let destination = state.destination();
        let mut samples = Vec::new();
        for now in c["samples"].as_array().unwrap() {
            state = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            let point: Vec<_> = (0..2)
                .map(|axis| {
                    state
                        .sample_axis(axis, now.as_u64().unwrap() as u32)
                        .unwrap()
                })
                .collect();
            samples.push(serde_json::json!({"point":point,"state":state}));
        }
        let before = state.clone();
        assert!(state.sample_axis(2, 0).is_err());
        assert_eq!(state, before);
        state.cancel();
        assert_eq!(
            serde_json::json!({"initial":initial,"destination":destination,"samples":samples,"cancelled":state}),
            row["original"],
            "motion {n}"
        );
    }
}
