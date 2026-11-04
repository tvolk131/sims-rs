use serde::Deserialize;
use serde_json::Value;
use sims_sim::lot_acceptance::{Floor, Input, Outcome, Reply, Request};

#[derive(Deserialize)]
struct Case {
    #[serde(flatten)]
    input: Input,
    initial_error: i32,
    active_level: i32,
    wall_fit: bool,
    wall_error: i32,
    collision: bool,
    collision_error: i32,
}
fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c: Case = serde_json::from_value(row["input"].clone()).unwrap();
        let mut op = Floor::new(c.input, c.initial_error).unwrap();
        let mut error = c.initial_error;
        let mut events = vec![];
        while let Some(request) = op.next() {
            // Every native service is a repeatable, serializable boundary.
            assert_eq!(op.next(), Some(request));
            let saved = serde_json::to_vec(&op).unwrap();
            op = serde_json::from_slice(&saved).unwrap();
            assert_eq!(op.next(), Some(request));
            let old = op.clone();
            let wrong = if request == Request::ActiveLevel {
                Reply::Geometry(Outcome {
                    accepted: true,
                    error,
                })
            } else {
                Reply::ActiveLevel(1)
            };
            assert!(op.resume(request, wrong).is_err());
            assert_eq!(op, old);
            events.push(request);
            let reply = match request {
                Request::Containment => panic!("floor probe requested containment"),
                Request::ActiveLevel => Reply::ActiveLevel(c.active_level),
                Request::WallFit | Request::Collision => {
                    let (accepted, failure) = if request == Request::WallFit {
                        (c.wall_fit, c.wall_error)
                    } else {
                        (c.collision, c.collision_error)
                    };
                    if !accepted {
                        error = failure;
                    }
                    Reply::Geometry(Outcome { accepted, error })
                }
            };
            op.resume(request, reply).unwrap();
        }
        let outcome = op.outcome().unwrap();
        assert_eq!(
            serde_json::json!({"accepted":outcome.accepted,"error":outcome.error,"events":events}),
            row["original"],
            "native case {i}: {}",
            row["input"]
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn floor_acceptance_native_branches_and_service_order() {
    compare(serde_json::from_str(include_str!("fixtures/floor-acceptance.json")).unwrap());
}

#[test]
#[ignore = "requires freshly generated private original floor acceptance report"]
fn all_original_floor_acceptance_cases() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/floor-acceptance.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[test]
#[ignore = "requires native report from the three original house spawn inputs"]
fn original_house_spawn_floor_inputs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-floor-acceptance.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}
