use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::lot_presenter::{
    centering, motion,
    motion::frame::{Cursor, Reply, Request},
    scroll,
};

#[derive(Serialize, Deserialize)]
struct Host {
    state: centering::State,
    motion: motion::State,
    attempts: u32,
    bottom_margin: i32,
    now: u32,
}
impl Host {
    fn scroll(&self) -> scroll::State {
        scroll::State {
            origin: self.state.origin,
            pending: self.state.pending,
            deferred: self.state.motion_allowed,
            locked: self.state.motion_disabled,
            attempts: self.attempts,
            bottom_margin: self.bottom_margin,
        }
    }
    fn publish_scroll(&mut self, s: scroll::State) {
        self.state.origin = s.origin;
        self.state.pending = s.pending;
        self.state.motion_allowed = s.deferred;
        self.state.motion_disabled = s.locked;
        self.attempts = s.attempts;
        self.bottom_margin = s.bottom_margin;
    }
    fn snapshot(&mut self) -> Value {
        self.motion.active = self.state.motion_active;
        self.motion.policy = self.state.motion_policy;
        self.motion.duration = self.state.motion_duration;
        json!({"state":self.state,"motion":self.motion,"scroll":self.scroll()})
    }
    fn event(&mut self, request: Value, c: &Value, events: &mut Vec<Value>) {
        let mut event = self.snapshot();
        event["request"] = request;
        let index = events.len();
        events.push(event);
        for change in c["changes"].as_array().unwrap() {
            if change["event"].as_u64().unwrap() as usize != index {
                continue;
            }
            if let Some(patch) = change["state"].as_object() {
                let mut state = serde_json::to_value(&self.state).unwrap();
                for (key, value) in patch {
                    state[key] = value.clone();
                }
                self.state = serde_json::from_value(state).unwrap();
            }
            if let Some(patch) = change["scroll"].as_object() {
                let mut state = serde_json::to_value(self.scroll()).unwrap();
                for (key, value) in patch {
                    state[key] = value.clone();
                }
                self.publish_scroll(serde_json::from_value(state).unwrap());
            }
            if let Some(now) = change["now"].as_u64() {
                self.now = now as u32;
            }
        }
    }
    fn renderer(&mut self, c: &Value, events: &mut Vec<Value>) {
        if c["renderer_ready"].as_bool().unwrap() {
            self.event(json!({"Renderer":{"zoom":self.state.zoom,"origin":self.state.origin,"rotation":c["rotation"]}}), c, events);
        }
    }
    fn service(&mut self, request: &Request, c: &Value, events: &mut Vec<Value>) -> Reply {
        let native = c["native"].as_bool().unwrap();
        match *request {
            Request::SampleMotion { axis } => {
                let value = if native {
                    match self
                        .motion
                        .center_service(
                            &mut self.state,
                            &centering::Request::SampleMotion { axis },
                            self.now,
                        )
                        .unwrap()
                    {
                        centering::Reply::Coordinate(value) => value,
                        _ => unreachable!(),
                    }
                } else {
                    c["sample"][usize::from(axis)].as_i64().unwrap() as i32
                };
                Reply::Coordinate(value)
            }
            Request::CancelMotion if native => {
                let mut cursor = motion::cancel::Cursor::begin(&mut self.state);
                while let Some(request) = cursor.request().cloned() {
                    // Retain the entire owner while a nested callback is suspended.
                    let bytes = serde_json::to_vec(&(&self, &cursor)).unwrap();
                    let (host, restored): (Self, motion::cancel::Cursor) =
                        serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(serde_json::to_vec(&(&host, &restored)).unwrap(), bytes);
                    *self = host;
                    cursor = restored;
                    self.event(json!({"CancelService":request}), c, events);
                    let reply = if request == motion::cancel::Request::ResolveSurface {
                        motion::cancel::Reply::Surface(7)
                    } else {
                        motion::cancel::Reply::Done
                    };
                    cursor.acknowledge(&request, reply).unwrap();
                }
                Reply::Done
            }
            Request::Scroll {
                position,
                immediate,
            } if native => {
                let mut state = self.scroll();
                let context: scroll::Context =
                    serde_json::from_value(c["context"].clone()).unwrap();
                let mut cursor = scroll::Cursor::begin(
                    &mut state,
                    &context,
                    position,
                    immediate,
                    c["renderer_ready"].as_bool().unwrap(),
                    c["rotation"].as_u64().unwrap() as u32,
                )
                .unwrap();
                self.publish_scroll(state);
                if let Some(request) = cursor.request().cloned() {
                    let bytes = serde_json::to_vec(&(&self, &cursor)).unwrap();
                    let (host, restored): (Self, scroll::Cursor) =
                        serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(serde_json::to_vec(&(&host, &restored)).unwrap(), bytes);
                    *self = host;
                    cursor = restored;
                    self.renderer(c, events);
                    cursor.acknowledge(&request).unwrap();
                }
                assert!(cursor.complete());
                Reply::Done
            }
            Request::Synchronize if native => {
                self.renderer(c, events);
                Reply::Done
            }
            _ => Reply::Done,
        }
    }
}

fn compare(text: &str) {
    let report: Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["entry"], "0x4a8b3e");
    assert_eq!(report["end"], "0x4a8bad");
    assert_eq!(report["cases"].as_array().unwrap().len(), 320);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut host = Host {
            state: serde_json::from_value(c["state"].clone()).unwrap(),
            motion: serde_json::from_value(c["motion"].clone()).unwrap(),
            attempts: c["scroll"]["attempts"].as_u64().unwrap() as u32,
            bottom_margin: c["scroll"]["bottom_margin"].as_i64().unwrap() as i32,
            now: c["now"].as_u64().unwrap() as u32,
        };
        let mut cursor = Cursor::begin(&mut host.state);
        let mut events = Vec::new();
        while let Some(request) = cursor.request().cloned() {
            let bytes = serde_json::to_vec(&(&host, &cursor)).unwrap();
            let restored: (Host, Cursor) = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
            (host, cursor) = restored;
            let wrong = if matches!(request, Request::SampleMotion { .. }) {
                Reply::Done
            } else {
                Reply::Coordinate(17)
            };
            assert!(
                cursor
                    .acknowledge(&mut host.state, &request, wrong)
                    .is_err()
            );
            assert_eq!(serde_json::to_vec(&(&host, &cursor)).unwrap(), bytes);
            host.event(serde_json::to_value(&request).unwrap(), c, &mut events);
            let reply = host.service(&request, c, &mut events);
            cursor
                .acknowledge(&mut host.state, &request, reply)
                .unwrap();
            assert!(
                cursor
                    .acknowledge(&mut host.state, &request, Reply::Done)
                    .is_err()
            );
        }
        assert!(cursor.complete());
        let mut actual = host.snapshot();
        actual["events"] = Value::from(events);
        assert_eq!(actual, row["original"], "camera frame case {n}");
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_camera_frame_order_and_live_callback_state() {
    compare(include_str!("fixtures/camera-frame-owner.json"));
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_camera_frame_uses_actual_curves_scrolling_and_cancellation() {
    compare(include_str!("fixtures/camera-frame-motion.json"));
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_camera_frame_displacement_consumption_and_draw_pass_order() {
    use motion::frame::displacement::{Cursor, Passes, Reply, Request};
    fn event(
        host: &mut Host,
        passes: &mut Passes,
        request: Value,
        c: &Value,
        events: &mut Vec<Value>,
    ) {
        let index = events.len();
        let before = serde_json::to_value(&passes).unwrap();
        host.event(request, c, events);
        events[index]["passes"] = before;
        for change in c["changes"].as_array().unwrap() {
            if change["event"].as_u64().unwrap() as usize == index {
                if let Some(patch) = change["passes"].as_object() {
                    let mut p = serde_json::to_value(&passes).unwrap();
                    for (key, value) in patch {
                        p[key] = value.clone();
                    }
                    *passes = serde_json::from_value(p).unwrap();
                }
            }
        }
    }
    let report: Value =
        serde_json::from_str(include_str!("fixtures/camera-displacement.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["entry"], "0x4a8d25");
    assert_eq!(report["end"], "0x4a8ddc");
    assert_eq!(report["cases"].as_array().unwrap().len(), 320);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut host = Host {
            state: serde_json::from_value(c["state"].clone()).unwrap(),
            motion: serde_json::from_value(c["motion"].clone()).unwrap(),
            attempts: c["scroll"]["attempts"].as_u64().unwrap() as u32,
            bottom_margin: c["scroll"]["bottom_margin"].as_i64().unwrap() as i32,
            now: c["now"].as_u64().unwrap() as u32,
        };
        let mut passes: Passes = serde_json::from_value(c["passes"].clone()).unwrap();
        let mut cursor = Cursor::begin(&host.state).unwrap();
        let mut events = vec![];
        while let Some(request) = cursor.request().cloned() {
            let bytes = serde_json::to_vec(&(&host, &passes, &cursor)).unwrap();
            let restored: (Host, Passes, Cursor) = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
            (host, passes, cursor) = restored;
            let wrong = if request == Request::DrawingEnabled {
                Reply::Done
            } else {
                Reply::Enabled(0)
            };
            assert!(
                cursor
                    .acknowledge(&mut host.state, &mut passes, &request, wrong)
                    .is_err()
            );
            assert_eq!(
                serde_json::to_vec(&(&host, &passes, &cursor)).unwrap(),
                bytes
            );
            event(
                &mut host,
                &mut passes,
                serde_json::to_value(&request).unwrap(),
                c,
                &mut events,
            );
            if request == Request::Synchronize && c["renderer_ready"].as_bool().unwrap() {
                let renderer = json!({"Renderer":{"zoom":host.state.zoom,"origin":host.state.origin,"rotation":c["rotation"]}});
                event(&mut host, &mut passes, renderer, c, &mut events);
            }
            let reply = if request == Request::DrawingEnabled {
                Reply::Enabled(c["enabled"].as_u64().unwrap() as u8)
            } else {
                Reply::Done
            };
            cursor
                .acknowledge(&mut host.state, &mut passes, &request, reply)
                .unwrap();
            assert!(
                cursor
                    .acknowledge(&mut host.state, &mut passes, &request, Reply::Done)
                    .is_err()
            );
        }
        assert!(cursor.complete());
        assert!(Cursor::begin(&host.state).is_err());
        let mut actual = host.snapshot();
        actual["events"] = Value::from(events);
        actual["passes"] = serde_json::to_value(passes).unwrap();
        assert_eq!(actual, row["original"], "camera displacement case {n}");
    }
}
