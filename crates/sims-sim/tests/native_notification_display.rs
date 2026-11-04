use serde_json::{Value, json};
use sims_sim::person_notification::display::{Cache, Execution, Request, Response};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_notification_render_lifetime() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-notification-lifetime.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 56);
    for row in rows {
        assert_eq!(
            sims_sim::person_notification::display::within_display_lifetime(
                row["cached"].as_u64().unwrap() as u32,
                row["now"].as_u64().unwrap() as u32
            ),
            row["visible"].as_bool().unwrap(),
            "{row}"
        );
    }
}

fn restore(execution: &mut Execution, cache: &mut Option<Cache>, color: &mut u32) {
    let bytes = serde_json::to_vec(&(&*execution, &*cache, *color)).unwrap();
    let restored: (Execution, Option<Cache>, u32) = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (&*execution, &*cache, *color),
        (&restored.0, &restored.1, restored.2)
    );
    (*execution, *cache, *color) = restored;
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_display_cache_service_order_and_resume() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-notification-display.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 768);
    for (case, row) in rows.iter().enumerate() {
        let c = &row["input"];
        let mut cache: Option<Cache> = serde_json::from_value(c["cache"].clone()).unwrap();
        let mut color = c["color"].as_u64().unwrap() as u32;
        let mut execution = Execution::new(
            serde_json::from_value(c["text"].clone()).unwrap(),
            c["background"].as_u64().unwrap() as u32,
            c["foreground"].as_u64().unwrap() as u32,
            c["font"].as_bool().unwrap(),
        )
        .unwrap();
        for (step, expected) in row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            restore(&mut execution, &mut cache, &mut color);
            let request = execution.next(&mut cache).unwrap().unwrap();
            assert_eq!(
                json!(request),
                expected["request"],
                "case {case} service {step}"
            );
            assert_eq!(json!(cache), expected["cache"], "case {case} cache {step}");
            assert_eq!(json!(color), expected["color"], "case {case} color {step}");
            assert!(!execution.is_complete());
            restore(&mut execution, &mut cache, &mut color);
            let before = (execution.clone(), cache.clone());
            assert_eq!(execution.next(&mut cache).unwrap(), Some(request.clone()));
            assert_eq!((execution.clone(), cache.clone()), before);
            if !matches!(request, Request::Measure { .. }) {
                assert!(execution.clone().next(&mut None).is_err());
            }
            let mut malformed = json!(execution);
            malformed["pending"] = json!({"Measure":{"text":[111]}});
            let mut malformed: Execution = serde_json::from_value(malformed).unwrap();
            assert!(malformed.next(&mut cache.clone()).is_err());
            let invalid = match request {
                Request::Clock => Response::Done,
                _ => Response::Clock(37),
            };
            assert!(execution.acknowledge(&mut cache, invalid).is_err());
            assert_eq!((execution.clone(), cache.clone()), before);
            let response = match request {
                Request::Measure { .. } => {
                    Response::Bounds(serde_json::from_value(c["measure"].clone()).unwrap())
                }
                Request::Allocate => Response::Bounds([0; 4]),
                Request::Create { size } => Response::Created {
                    bounds: [0, 0, size[0], size[1]],
                    success: c["created"].as_bool().unwrap(),
                },
                Request::Clock => Response::Clock(c["now"].as_u64().unwrap() as u32),
                Request::Lock => Response::Bool(c["locked"].as_bool().unwrap()),
                Request::Fill { .. } => Response::Bool(c["filled"].as_bool().unwrap()),
                Request::FontColor => Response::Color(color),
                Request::SetFontColor { color: next } => {
                    color = next;
                    Response::Done
                }
                _ => Response::Done,
            };
            execution.acknowledge(&mut cache, response).unwrap();
            restore(&mut execution, &mut cache, &mut color);
        }
        assert_eq!(
            execution.next(&mut cache).unwrap(),
            None,
            "case {case} trailing request"
        );
        assert!(execution.is_complete());
        assert_eq!(
            json!(cache),
            row["original"]["cache"],
            "case {case} final cache"
        );
        assert_eq!(
            json!(color),
            row["original"]["color"],
            "case {case} final color"
        );
        restore(&mut execution, &mut cache, &mut color);
    }
}
