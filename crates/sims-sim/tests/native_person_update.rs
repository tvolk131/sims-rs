use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::person_update::{Context, Execution, Request, Response, State};
use sims_vm::{
    Object,
    motives::{Change, UpdateState},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Host {
    object: Object,
    state: State,
    context: Context,
    statistics: [u32; 2],
}
impl Host {
    fn patch(&mut self, p: &Value) {
        if let Some(fields) = p["person"].as_object() {
            for (key, value) in fields {
                self.object
                    .person
                    .as_mut()
                    .unwrap()
                    .insert(key.parse().unwrap(), value.as_i64().unwrap() as i16);
            }
        }
        if let Some(values) = p["values"].as_object() {
            for (key, value) in values {
                self.object.motives.as_mut().unwrap()[key.parse::<usize>().unwrap()] =
                    value.as_u64().unwrap() as u32;
            }
        }
        if let Some(hidden) = p["hidden"].as_i64() {
            self.object.data.insert(34, hidden as i16);
        }
        if let Some(room) = p["room"].as_i64() {
            self.state.room = room as i16;
        }
        if let Some(last) = p["last_decay"].as_i64() {
            self.object.motive_update.as_mut().unwrap().last_decay = last as i32;
        }
        if !p["displayed_animation"].is_null() {
            self.state.displayed_animation =
                serde_json::from_value(p["displayed_animation"].clone()).unwrap();
        }
        if let Some(changes) = p["changes"].as_array() {
            self.object.motive_changes = changes
                .iter()
                .map(|c| Change {
                    motive: c[0].as_u64().unwrap() as u8,
                    delta: c[1].as_u64().unwrap() as u32,
                    target: c[2].as_u64().unwrap() as u32,
                })
                .collect();
        }
        for (key, field) in [
            ("only_visible", &mut self.context.only_visible),
            (
                "display_animation_names",
                &mut self.context.display_animation_names,
            ),
            ("debug_recording", &mut self.context.debug_recording),
        ] {
            if let Some(value) = p[key].as_bool() {
                *field = value;
            }
        }
    }
    fn snapshot(&self) -> Value {
        let fields: std::collections::BTreeMap<_, _> = [1, 27, 29, 32, 85]
            .map(|i| (i, self.object.person.as_ref().unwrap()[&i]))
            .into();
        json!({"person":fields,"hidden":self.object.data[&34],"room":self.state.room,
            "last_decay":self.object.motive_update.as_ref().unwrap().last_decay,
            "values":self.object.motives.unwrap(),"statistics":self.statistics,
            "displayed_animation":self.state.displayed_animation})
    }
}
fn restore(execution: &mut Execution, host: &mut Host) {
    let bytes = serde_json::to_vec(&(&*execution, &*host)).unwrap();
    let (saved, saved_host) = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(*execution, saved);
    assert_eq!(*host, saved_host);
    (*execution, *host) = (saved, saved_host);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_person_update_service_order_and_resume() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-person-update.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let mut requests = 0;
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut host = Host {
            object: Object {
                person: Some(Default::default()),
                motives: Some([0; 16]),
                motive_update: Some(UpdateState {
                    previous: [0; 16],
                    last_decay: 0,
                }),
                ..Default::default()
            },
            state: State {
                room: 0,
                displayed_animation: vec![],
            },
            context: serde_json::from_value(c["context"].clone()).unwrap(),
            statistics: serde_json::from_value(c["statistics"].clone()).unwrap(),
        };
        host.patch(&c["initial"]);
        let mut e = Execution::new(c["tick"].as_i64().unwrap() as i32);
        for (step, native) in row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            restore(&mut e, &mut host);
            let request = e
                .next(&mut host.object, &mut host.state, host.context)
                .unwrap()
                .unwrap();
            assert_eq!(
                json!(request),
                native["request"],
                "case {case} request {step}"
            );
            assert_eq!(host.snapshot(), native["state"], "case {case} state {step}");
            restore(&mut e, &mut host);
            let before = (e.clone(), host.clone());
            // Polling or an invalid response cannot repeat effects or drop work.
            assert_eq!(
                e.next(&mut host.object, &mut host.state, host.context)
                    .unwrap(),
                Some(request.clone())
            );
            assert!(
                e.acknowledge(
                    &mut host.object,
                    &mut host.state,
                    if matches!(request, Request::ObjectUpdate { .. }) {
                        Response::Done
                    } else {
                        Response::ObjectUpdated(1)
                    }
                )
                .is_err()
            );
            assert_eq!((e.clone(), host.clone()), before);
            let key = if let Some(name) = native["request"].as_str() {
                name
            } else {
                native["request"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .next()
                    .unwrap()
            };
            host.patch(&c["patches"][key]);
            let response = match request {
                Request::ObjectUpdate { .. } => {
                    Response::ObjectUpdated(c["result"].as_u64().unwrap() as u8)
                }
                Request::Environment { .. } => {
                    Response::Environment((c["environment"].as_f64().unwrap() as f32).to_bits())
                }
                Request::AllowNegativeOutdoorEnvironment => {
                    Response::Bool(c["allow_negative"].as_bool().unwrap())
                }
                Request::HasActionUse => Response::Bool(c["has_action"].as_bool().unwrap()),
                Request::AnimationName => Response::AnimationName(
                    serde_json::from_value(c["animation_name"].clone()).unwrap(),
                ),
                Request::Statistics { has_action } => {
                    host.statistics[0] = host.statistics[0].wrapping_add(1);
                    if has_action {
                        host.statistics[1] = host.statistics[1].wrapping_add(1);
                    }
                    Response::Done
                }
                _ => Response::Done,
            };
            e.acknowledge(&mut host.object, &mut host.state, response)
                .unwrap();
            restore(&mut e, &mut host);
            requests += 1;
        }
        assert!(
            e.next(&mut host.object, &mut host.state, host.context)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            json!(e.result().unwrap()),
            row["original"]["result"],
            "case {case} result"
        );
        assert_eq!(
            host.snapshot(),
            row["original"]["state"],
            "case {case} final"
        );
        let before = (e.clone(), host.clone());
        assert!(
            e.next(&mut host.object, &mut host.state, host.context)
                .unwrap()
                .is_none()
        );
        assert_eq!((e, host), before);
    }
    println!("320 native person-update cases; {requests} saved service boundaries");
}

#[cfg(feature = "private-fixtures")]
#[test]
fn person_update_runs_original_motive_arithmetic_at_decay_boundary() {
    // This earlier native probe runs actual decay and per-tick changes, with
    // animator/membership supplied and room -5. It complements the new envelope
    // oracle's mutating service responses; neither is a live-household oracle.
    let report: Value =
        serde_json::from_str(include_str!("../../sims-vm/tests/fixtures/motives.json")).unwrap();
    let floats = |v: &Value| {
        serde_json::from_value::<[f32; 16]>(v.clone())
            .unwrap()
            .map(f32::to_bits)
    };
    for (i, row) in report["updates"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut fields: Vec<i16> = serde_json::from_value(c["person"].clone()).unwrap();
        fields[27] = c["counter"].as_i64().unwrap() as i16;
        fields[29] = c["decay_disabled"].as_i64().unwrap() as i16;
        fields[32] = 1; // Native probe's explicit statistics gate.
        let mut object = Object {
            person: Some(
                fields
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| (i as i16, v))
                    .collect(),
            ),
            motives: Some(floats(&c["values"])),
            motive_update: Some(UpdateState {
                previous: floats(&c["previous"]),
                last_decay: c["last_decay"].as_i64().unwrap() as i32,
            }),
            motive_changes: c["changes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Change {
                    motive: v[0].as_u64().unwrap() as u8,
                    delta: (v[1].as_f64().unwrap() as f32).to_bits(),
                    target: (v[2].as_f64().unwrap() as f32).to_bits(),
                })
                .collect(),
            ..Default::default()
        };
        object.data.insert(34, c["hidden"].as_i64().unwrap() as i16);
        let curves =
            serde_json::from_value::<Vec<sims_vm::motives::Curve>>(c["curves"].clone()).unwrap();
        let tuning = sims_vm::motives::Tuning {
            update_precision: Default::default(),
            rates: serde_json::from_value(c["rates"].clone()).unwrap(),
            adult_mood: curves.clone(),
            child_mood: curves,
            adult_scores: vec![],
            child_scores: vec![],
        };
        let mut state = State {
            room: -5,
            displayed_animation: vec![],
        };
        let context = Context {
            only_visible: true,
            display_animation_names: false,
            debug_recording: false,
        };
        let mut e = Execution::new(c["tick"].as_i64().unwrap() as i32);
        while let Some(request) = e.next(&mut object, &mut state, context).unwrap() {
            (e, object, state) =
                serde_json::from_slice(&serde_json::to_vec(&(&e, &object, &state)).unwrap())
                    .unwrap();
            match request {
                Request::ObjectUpdate { .. } => e
                    .acknowledge(
                        &mut object,
                        &mut state,
                        Response::ObjectUpdated(u8::from(c["complete"].as_bool().unwrap())),
                    )
                    .unwrap(),
                Request::Decay => e
                    .decay(
                        &mut object,
                        &mut state,
                        c["hour"].as_i64().unwrap() as i16,
                        &tuning,
                    )
                    .unwrap(),
                Request::Animator { .. } | Request::RoomMembership => e
                    .acknowledge(&mut object, &mut state, Response::Done)
                    .unwrap(),
                other => panic!("unexpected service {other:?} in motive case {i}"),
            }
        }
        assert_eq!(
            json!(object.motives.unwrap()),
            row["original"]["values"],
            "case {i}"
        );
        assert_eq!(
            json!(object.motive_update.as_ref().unwrap().previous),
            row["original"]["previous"],
            "case {i}"
        );
        assert_eq!(
            json!(object.motive_update.as_ref().unwrap().last_decay),
            row["original"]["last_decay"],
            "case {i}"
        );
        assert_eq!(
            json!(object.person.as_ref().unwrap()[&27]),
            row["original"]["counter"],
            "case {i}"
        );
    }
}
