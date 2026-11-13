use serde_json::{Value, json};
use sims_formats::{
    animation::{Animation, Motion, Samples},
    avatar::{Bone, Skeleton},
};
use sims_sim::pose::{
    animator::Animator,
    layer::{Layer, MotionInput, Start},
    movement::{self, Movement, Request},
    ordinary,
    transform::Placement,
};
use sims_vm::{Program, program::AnimationAsset};

pub fn compare(a: &Value, b: &Value, path: &str) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                compare(
                    a.get(k).unwrap_or_else(|| panic!("missing {path}.{k}")),
                    v,
                    &format!("{path}.{k}"),
                );
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{i}]"));
            }
        }
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            let a = a.as_f64().unwrap() as f32;
            let b = b.as_f64().unwrap() as f32;
            // One final rounding can differ between native x87 and f64 trig.
            assert!((a - b).abs() <= 0.00003, "{path}: {a:?} / {b:?}");
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}
fn project(m: &Movement, a: &Animator, p: &Placement, o: &ordinary::State, f: i16) -> Value {
    json!({"state":m.state,"path":m.path,"animator":a,"placement":p,"ordinary":o,"facing":f})
}
fn restore(
    m: &mut Movement,
    a: &mut Animator,
    p: &mut Placement,
    o: &mut ordinary::State,
    f: &mut i16,
) {
    let bytes = serde_json::to_vec(&(&*m, &*a, &*p, &*o, &*f)).unwrap();
    let saved: (Movement, Animator, Placement, ordinary::State, i16) =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (&*m, &*a, &*p, &*o, *f),
        (&saved.0, &saved.1, &saved.2, &saved.3, saved.4)
    );
    (*m, *a, *p, *o, *f) = saved;
}
fn animator(v: &Value, duration: f32) -> Animator {
    let mut a = json!(Animator::new(0, 0, 0, true, -1));
    for (k, v) in v.as_object().unwrap() {
        a[k] = v.clone();
    }
    for owned in a["layers"].as_array_mut().unwrap() {
        let mut layer = json!(
            Layer::new(
                "synthetic".into(),
                duration,
                &[MotionInput {
                    bone_id: 0,
                    frames: 30,
                    events: 0
                }],
                &[Some(0)],
                Start {
                    priority: 1,
                    speed: 1.,
                    weight: 1.,
                    now: 123400
                }
            )
            .unwrap()
        );
        for (k, v) in owned["layer"].as_object().unwrap() {
            layer[k] = v.clone();
        }
        owned["layer"] = layer;
        owned["anchor_bone"] = Value::Null;
        owned["anchor_state"] = json!(0);
    }
    serde_json::from_value(a).unwrap()
}
fn resources(duration: f32, distance: f32) -> (Skeleton, Program) {
    let skeleton = Skeleton {
        name: "synthetic".into(),
        bones: vec![Bone {
            name: "bone".into(),
            parent: None,
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            properties: vec![],
            flags: [0; 3],
            wiggle: Default::default(),
        }],
    };
    let clip = Animation {
        name: "synthetic".into(),
        skill: "synthetic".into(),
        duration_ms: duration,
        distance,
        moving: true,
        translation_count: 0,
        rotation_count: 0,
        frames: 30,
        events: vec![],
        motions: vec![Motion {
            bone: "bone".into(),
            frames: 30,
            duration_ms: duration,
            translation: None,
            rotation: None,
            properties: vec![],
            event_lists: vec![],
        }],
    };
    let mut program = Program::default();
    program.animations.insert(
        "synthetic".into(),
        AnimationAsset {
            source: "authored movement probe".into(),
            content_hash: "synthetic".into(),
            samples_source: "synthetic".into(),
            samples_hash: "synthetic".into(),
            clip,
            samples: Samples {
                translations: vec![],
                rotations: vec![],
            },
        },
    );
    (skeleton, program)
}
pub fn run(report: Value) -> (usize, usize, usize) {
    let settings: movement::Settings = serde_json::from_value(report["settings"].clone()).unwrap();
    let mut count = 0;
    let mut services = 0;
    let mut paths = 0;
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let b = &row["before"];
        let duration = c["duration"].as_f64().unwrap() as f32;
        let (skeleton, mut program) = resources(duration, c["distance"].as_f64().unwrap() as f32);
        program.animations.get_mut("synthetic").unwrap().clip.moving =
            c["clip_moving"].as_bool().unwrap_or(true);
        let layers = serde_json::from_value(c["layers"].clone()).unwrap();
        let mut a = animator(&b["animator"], duration);
        let mut m = Movement::new(serde_json::from_value(b["state"].clone()).unwrap());
        if !b["path"].is_null() {
            m.path = Some(serde_json::from_value(b["path"].clone()).unwrap());
        }
        let mut p: Placement = serde_json::from_value(b["placement"].clone()).unwrap();
        let mut o: ordinary::State = serde_json::from_value(b["ordinary"].clone()).unwrap();
        let mut f = b["facing"].as_i64().unwrap() as i16;
        let entry = ordinary::Entry {
            root_motion: c["root_motion"].as_u64().unwrap_or(0) as u8,
            movement_input: c["movement_input"].as_i64().unwrap_or(0) as i16,
            walk_style: c["source"]["walk_style"].as_i64().unwrap() as i16,
        };
        m.begin(
            2468,
            ordinary::Continuation::Movement {
                now: 123400,
                step: 42,
                entry,
            },
        )
        .unwrap();
        let mut source = movement::Source {
            object9: c["object9"].as_i64().unwrap_or(0) as i16,
            level: c["level"].as_i64().unwrap_or(1) as i32,
            species: c["source"]["species"].as_i64().unwrap() as i16,
            person78: c["source"]["person78"].as_i64().unwrap() as i16,
            walk_style: entry.walk_style,
        };
        let mut events = 0;
        loop {
            restore(&mut m, &mut a, &mut p, &mut o, &mut f);
            let Some(request) = m
                .next(
                    &mut a, &mut p, &mut o, &mut f, source, settings, layers, &program,
                )
                .unwrap_or_else(|e| panic!("case {case}: {e}"))
            else {
                break;
            };
            let event = &row["events"][events];
            assert_eq!(
                json!(request),
                event["request"],
                "case {case} service {events}"
            );
            compare(
                &project(&m, &a, &p, &o, f),
                &event["snapshot"],
                &format!("case {case} service {events}"),
            );
            let before = (&m, &a, &p, &o, f);
            let bytes = serde_json::to_vec(&before).unwrap();
            assert_eq!(
                m.next(
                    &mut a, &mut p, &mut o, &mut f, source, settings, layers, &program
                )
                .unwrap(),
                Some(request)
            );
            let wrong = if request == Request::Placement {
                Request::BasePosture
            } else {
                Request::Placement
            };
            assert!(m.serviced(wrong, &mut a).is_err());
            assert_eq!(bytes, serde_json::to_vec(&(&m, &a, &p, &o, f)).unwrap());
            restore(&mut m, &mut a, &mut p, &mut o, &mut f);
            match request {
                Request::Foreground { index } => {
                    if c["mutate_lookup"].as_bool().unwrap() {
                        let mut saved = json!(a);
                        saved["selection_cache"] = json!(index);
                        a = serde_json::from_value(saved).unwrap();
                    }
                    let clip = (c["present"].as_bool().unwrap() && c["ready"].as_bool().unwrap())
                        .then_some(&program.animations["synthetic"].clip);
                    m.selected(&mut a, &o, &skeleton, clip, layers).unwrap();
                }
                Request::RootDisplacement { .. } => m
                    .displacement(
                        &mut p,
                        &mut f,
                        serde_json::from_value(c["displacement"].clone()).unwrap(),
                        settings,
                    )
                    .unwrap(),
                Request::BasePosture => {
                    if let Some(posture) = c["posture"].as_i64() {
                        let input = sims_sim::pose::posture::Input {
                            posture: posture as i16,
                            species: source.species,
                            definition_flags: c["definition_flags"].as_u64().unwrap() as u8,
                            tick: 2468,
                        };
                        let predicted = sims_sim::pose::posture::selection(&a, &o, input);
                        let index = sims_sim::pose::posture::prepare(&mut a, &mut o, input, layers)
                            .unwrap();
                        assert_eq!(predicted, index);
                        if let Some(index) = index {
                            let lookups = row["base_lookups"].as_array().unwrap();
                            assert_eq!(lookups.len(), 1);
                            assert_eq!(lookups[0]["index"], index);
                            compare(
                                &project(&m, &a, &p, &o, f),
                                &lookups[0]["snapshot"],
                                &format!("case {case} base lookup"),
                            );
                            restore(&mut m, &mut a, &mut p, &mut o, &mut f);
                            let clip = c["base_present"]
                                .as_bool()
                                .unwrap()
                                .then_some(&program.animations["synthetic"].clip);
                            sims_sim::pose::posture::selected(
                                &mut a, &o, input.tick, &skeleton, clip, true, layers,
                            )
                            .unwrap();
                        } else {
                            assert_eq!(row["base_lookups"], json!([]));
                        }
                    } else {
                        // Older movement-only corpus supplies 590430 itself.
                        o.base_posture = 321;
                    }
                    m.serviced(request, &mut a).unwrap();
                }
                Request::Placement => m.serviced(request, &mut a).unwrap(),
                Request::StopForeground { .. } => {
                    let clip = (c["present"].as_bool().unwrap() && c["ready"].as_bool().unwrap())
                        .then_some(&program.animations["synthetic"].clip);
                    m.stop_selected(&mut a, &o, &skeleton, clip, layers)
                        .unwrap();
                }
                Request::AcceptStep { .. } => {
                    if c["mutate_accept"].as_bool().unwrap() {
                        source.level = 2;
                    }
                    m.accept_step(c["accepted"].as_bool().unwrap()).unwrap();
                }
                Request::MoveStep { .. } => {
                    if c["mutate_move"].as_bool().unwrap() {
                        m.path.as_mut().unwrap().interrupt = 1;
                        m.state.status = 1;
                        let mut saved = json!(a);
                        let id = saved["slots"][0].clone();
                        for owned in saved["layers"].as_array_mut().unwrap() {
                            if owned["id"] == id {
                                owned["layer"]["interruptible"] = json!(1);
                            }
                        }
                        a = serde_json::from_value(saved).unwrap();
                    }
                    m.moved(&mut a, &mut p, &mut o).unwrap();
                }
            }
            events += 1;
            services += 1;
        }
        assert_eq!(events, row["events"].as_array().unwrap().len());
        compare(
            &project(&m, &a, &p, &o, f),
            &row["original"],
            &format!("case {case} final"),
        );
        let outcome = m.finish().unwrap();
        if row["continuation"] == "Path" {
            paths += 1;
            assert_eq!(
                outcome,
                movement::Continuation::Path {
                    now: 123400,
                    step: 42,
                    entry
                }
            );
        } else {
            let entry = ordinary::Entry {
                root_motion: row["root_motion"]
                    .as_u64()
                    .map_or(entry.root_motion, |v| v as u8),
                ..entry
            };
            assert_eq!(outcome, movement::Continuation::Tail { now: 123400, entry });
        }
        count += 1;
    }
    (count, services, paths)
}
