//! Compare routing decisions with original executable routines on synthetic worlds.
use serde_json::{Value, json};
use sims_sim::{Point, routing};

fn point(value: &Value) -> Point {
    Point::new(
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
    )
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("private routing report path");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    compare(report);
}

pub fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = report["departures"].as_array().unwrap();
    for (index, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let mut checks = Vec::new();
        let allowed = if input["contained"] == true && input["sit_function"] == true {
            input["mask"].as_u64().unwrap() as u16
        } else {
            0
        };
        let result = routing::select_departure(
            point(&input["position"]),
            point(&input["target"]),
            input["facing"].as_i64().unwrap() as i16,
            allowed,
            |p, direction| {
                let classification = input["classification"][usize::from(direction)]
                    .as_i64()
                    .unwrap() as i32;
                checks.push(json!({"point": [p.x, p.y], "direction": direction, "classification": classification}));
                classification
            },
        );
        let actual = json!({"position": [result.position.x, result.position.y], "flags": result.flags, "checks": checks, "error": null});
        assert_eq!(actual, row["original"], "departure case {index}: {input}");
    }
    println!("{} original seat-departure comparisons passed", rows.len());
    let rows = report["candidates"].as_array().unwrap();
    for (index, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let request: routing::StandingRequest = serde_json::from_value(input.clone()).unwrap();
        let mut rng = input["seed"].as_u64().unwrap() as u32;
        let mut checks = Vec::new();
        let mut result = routing::standing_candidates(&request, &mut rng, |p, direction| {
            let classification = input["classification"][usize::from(direction)].as_i64().unwrap() as i32;
            checks.push(json!({"point": [p.x, p.y], "direction": direction, "classification": classification}));
            classification
        }).unwrap();
        if let Some(limit) = input["limit"].as_u64() {
            routing::limit_candidates(&mut result, limit as usize, &mut rng);
        }
        let mut actual = serde_json::to_value(result).unwrap();
        actual["rng"] = json!(rng);
        actual["checks"] = json!(checks);
        actual["error"] = Value::Null;
        assert_eq!(actual, row["original"], "candidate case {index}: {input}");
    }
    println!(
        "{} original candidate generation comparisons passed",
        rows.len()
    );
}
