use sims_vm::interface::{Inputs, LiveModeUpdate, Request};

fn inputs() -> Inputs {
    Inputs {
        menu_present: false,
        menu_active: 0,
        closing: 0,
        mode: 2,
        paused: 0,
        include_visible: 7,
    }
}
fn restore(call: &LiveModeUpdate) -> LiveModeUpdate {
    let c: LiveModeUpdate = serde_json::from_slice(&serde_json::to_vec(call).unwrap()).unwrap();
    c.validate().unwrap();
    c
}

#[test]
fn mode_publication_follows_callbacks_and_every_pending_request_restores() {
    for paused in [0, 1, 128, 255] {
        let input = Inputs { paused, ..inputs() };
        let mut call = LiveModeUpdate::default();
        let mut requests = vec![];
        let mut global19 = 3;
        while let Some(request) = call.advance(input).unwrap() {
            let mut resumed = restore(&call);
            assert_eq!(resumed.advance(input).unwrap(), Some(request.clone()));
            if request == Request::PublishLiveMode {
                assert_eq!(global19, 3);
                assert_eq!(requests[0..2], [Request::RoomMask(7), Request::DrawingMode]);
                global19 = 0;
            }
            call.acknowledge(&request).unwrap();
            resumed.acknowledge(&request).unwrap();
            assert_eq!(call, resumed);
            requests.push(request);
        }
        assert!(call.complete());
        assert_eq!(global19, 0);
        assert_eq!(requests.contains(&Request::ResumeAudio), paused == 0);
        assert_eq!(
            requests[requests.len() - 2..],
            [Request::StopAudio, Request::PlayLiveBackground]
        );
    }
}

#[test]
fn gates_are_rechecked_at_native_positions_not_at_each_resume() {
    for input in [
        Inputs {
            menu_present: true,
            ..inputs()
        },
        Inputs {
            closing: 1,
            mode: -1,
            ..inputs()
        },
    ] {
        let mut call = LiveModeUpdate::default();
        assert_eq!(call.advance(input).unwrap(), None);
        assert!(call.complete());
    }
    let mut call = LiveModeUpdate::default();
    let mut input = inputs();
    while let Some(request) = call.advance(input).unwrap() {
        call = restore(&call);
        call.acknowledge(&request).unwrap();
        if request == Request::RoomMask(7) {
            // A closing change after entry does not suppress later publication.
            input.closing = 1;
        }
        if request == Request::PublishLiveMode {
            input.menu_present = true;
            break;
        }
    }
    assert_eq!(call.advance(input).unwrap(), None);
    assert!(call.complete());

    // The final menu guard precedes StopAudio, not PlayLiveBackground.
    let mut call = LiveModeUpdate::default();
    while let Some(request) = call.advance(inputs()).unwrap() {
        call.acknowledge(&request).unwrap();
        if request == Request::StopAudio {
            break;
        }
    }
    assert_eq!(
        call.advance(input).unwrap(),
        Some(Request::PlayLiveBackground)
    );
}

#[test]
fn unrelated_modes_and_wrong_replies_stay_explicit() {
    let mut call = LiveModeUpdate::default();
    let before = call.clone();
    assert!(
        call.advance(Inputs {
            mode: 0,
            ..inputs()
        })
        .is_err()
    );
    assert_eq!(call, before);
    let request = call.advance(inputs()).unwrap().unwrap();
    let before = call.clone();
    assert!(call.acknowledge(&Request::PublishLiveMode).is_err());
    assert_eq!(call, before);
    call.acknowledge(&request).unwrap();
    assert!(
        call.advance(Inputs {
            mode: 3,
            ..inputs()
        })
        .is_err()
    );
    assert!(!call.complete());
}

#[test]
#[ignore = "private original4ce44f execution corpus; run native_interface_mode.py with no Cargo/rustc"]
fn live_mode_matches_original_instructions() {
    use serde_json::{Value, json};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/interface-live-mode.json");
    let corpus: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in corpus["cases"].as_array().unwrap() {
        let mut fields = row["input"].clone();
        let mut call = LiveModeUpdate::default();
        let mut events = vec![];
        let mut global19 = 3;
        while let Some(request) = call
            .advance(serde_json::from_value(fields.clone()).unwrap())
            .unwrap()
        {
            call = restore(&call);
            events.push(json!({"request":request,"global19":global19}));
            if request == Request::PublishLiveMode {
                global19 = 0;
            }
            let key = match &request {
                Request::RoomMask(_) => "RoomMask",
                Request::DrawingMode => "DrawingMode",
                Request::ResumeAudio => "ResumeAudio",
                Request::PublishLiveMode => "PublishLiveMode",
                Request::StopAudio => "StopAudio",
                Request::PlayLiveBackground => "PlayLiveBackground",
            };
            if let Some(changes) = row["mutations"][key].as_object() {
                for (key, value) in changes {
                    fields[key] = value.clone();
                }
            }
            call.acknowledge(&request).unwrap();
        }
        assert_eq!(
            json!({"events":events,"global19":global19}),
            row["original"],
            "{row}"
        );
    }
}
