use serde_json::{Value, json};
use sims_sim::person_notification::{Execution, Input, Request, Response, View};

fn restore(execution: &mut Execution) {
    let bytes = serde_json::to_vec(execution).unwrap();
    let saved: Execution = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(*execution, saved);
    *execution = saved;
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_notification_dispatch_and_resume() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-person-notification.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 288);
    for (case, row) in rows.iter().enumerate() {
        let c = &row["input"];
        let mut execution = Execution::new(
            serde_json::from_value(c["input"].clone()).unwrap(),
            serde_json::from_value(c["view"].clone()).unwrap(),
        )
        .unwrap();
        for (step, expected) in row["original"].as_array().unwrap().iter().enumerate() {
            restore(&mut execution);
            let request = execution.next().unwrap();
            assert_eq!(json!(request), *expected, "case {case} service {step}");
            assert!(!execution.is_complete());
            restore(&mut execution);
            let pending = execution.clone();
            assert_eq!(execution.next(), Some(request.clone()));
            assert_eq!(execution, pending);
            let invalid = match request {
                Request::FormatMoney { .. } => Response::Done,
                _ => Response::Text(b"incorrect".to_vec()),
            };
            assert!(execution.acknowledge(invalid).is_err());
            assert_eq!(execution, pending);
            let response = match request {
                Request::FormatMoney { .. } => {
                    assert!(execution.acknowledge(Response::Text(vec![0])).is_err());
                    assert_eq!(execution, pending);
                    Response::Text(serde_json::from_value(c["localized"].clone()).unwrap())
                }
                _ => Response::Done,
            };
            execution.acknowledge(response).unwrap();
            restore(&mut execution);
        }
        assert_eq!(execution.next(), None, "case {case} trailing request");
        assert!(execution.is_complete());
        assert!(execution.acknowledge(Response::Done).is_err());
    }
}

#[test]
fn animation_label_requires_owner_even_without_presenter() {
    assert!(Execution::new(Input::AnimationName(b"walk".to_vec()), View::NoOwner).is_err());
    assert!(Execution::new(Input::AnimationName(vec![0]), View::Active).is_err());
    let mut suppressed =
        Execution::new(Input::AnimationName(b"walk".to_vec()), View::NoPresenter).unwrap();
    assert_eq!(suppressed.next(), None);
    assert!(suppressed.is_complete());
}
