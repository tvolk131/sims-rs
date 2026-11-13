use serde_json::{Value, json};
use sims_vm::autonomy::{Candidate, Enumeration, Selection, select};
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("private autonomy report path");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}
pub fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let policy: Selection = serde_json::from_value(c.clone()).unwrap();
        let mut candidates: Vec<Candidate> =
            serde_json::from_value(c["candidates"].clone()).unwrap();
        let mut rng = c["rng"].as_u64().unwrap() as u32;
        let selected = select(&mut candidates, &policy, &mut rng).unwrap();
        let original = &row["original"];
        // Compare typed f32 values, avoiding JSON decimal spelling differences.
        let ordered: Vec<Candidate> = serde_json::from_value(original["ordered"].clone()).unwrap();
        assert_eq!(candidates, ordered, "ordered case {i}");
        assert_eq!(
            json!(selected.is_some()),
            original["selected"],
            "selected case {i}"
        );
        assert_eq!(
            json!(selected.as_ref().map_or(-5, |s| s.object)),
            original["target"],
            "target case {i}"
        );
        assert_eq!(
            json!(selected.map_or(777, |s| s.menu as i16)),
            original["menu"],
            "menu case {i}"
        );
        assert_eq!(json!(rng), original["rng"], "random case {i}");
        assert_eq!(original["error"], Value::Null);
    }
    if let Some(rows) = report["enumeration"].as_array() {
        for (i, row) in rows.iter().enumerate() {
            let c = &row["input"];
            let policy: Enumeration = serde_json::from_value(c.clone()).unwrap();
            let eligible: Vec<u64> = c["entries"]
                .as_array()
                .unwrap()
                .iter()
                .rev()
                .filter(|e| {
                    policy
                        .permits(
                            e["flags"].as_u64().unwrap() as u32,
                            e["threshold"].as_i64().unwrap() as i16,
                        )
                        .unwrap()
                })
                .map(|e| e["id"].as_u64().unwrap())
                .collect();
            assert_eq!(
                json!(eligible),
                row["original"]["eligible"],
                "enumeration {i}"
            );
            assert_eq!(row["original"]["error"], Value::Null);
        }
        println!(
            "{} original autonomy enumeration comparisons passed",
            rows.len()
        );
    }
    for row in report["distances"].as_array().into_iter().flatten() {
        let c = &row["input"];
        let a = serde_json::from_value(c["a"].clone()).unwrap();
        let b = serde_json::from_value(c["b"].clone()).unwrap();
        assert_eq!(
            json!(
                (sims_vm::autonomy::distance(a, b, c["kind"].as_i64().unwrap() as i16) as f32)
                    .to_bits()
            ),
            row["original"]["bits"],
            "{c}"
        );
    }
    for row in report["attenuation"].as_array().into_iter().flatten() {
        let c = &row["input"];
        let tuning = sims_vm::autonomy::Tuning {
            family_threshold: 0.,
            visitor_threshold: 0.,
            sitting_threshold: 0.,
            random_count: 1,
            family_attenuation: serde_json::from_value(c["family_attenuation"].clone()).unwrap(),
            visitor_attenuation: serde_json::from_value(c["visitor_attenuation"].clone()).unwrap(),
        };
        assert_eq!(
            json!(
                tuning
                    .attenuation(
                        c["code"].as_u64().unwrap() as u32,
                        c["explicit"].as_f64().unwrap() as f32,
                        c["visitor"].as_bool().unwrap()
                    )
                    .to_bits()
            ),
            row["original"]["bits"],
            "{c}"
        );
    }
    println!(
        "{} original autonomy selection comparisons passed",
        report["cases"].as_array().unwrap().len()
    );
}
