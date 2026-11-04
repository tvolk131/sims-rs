use serde_json::{Value, json};
use sims_vm::motives::{self, Advertisement, Change, Curve, Rates, Tuning, UpdateState};

fn main() {
    let path = std::env::args().nth(1).expect("private motive report path");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}
fn floats(v: &Value) -> [u32; 16] {
    let values: [f32; 16] = serde_json::from_value(v.clone()).unwrap();
    values.map(f32::to_bits)
}
fn person(v: &Value) -> [i16; 256] {
    serde_json::from_value::<Vec<i16>>(v.clone())
        .unwrap()
        .try_into()
        .unwrap()
}
pub fn compare(report: Value) {
    let precision = report
        .get("update_precision")
        .map(|v| serde_json::from_value(v.clone()).unwrap())
        .unwrap_or_default();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["decay"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut values = floats(&c["values"]);
        let mut previous = floats(&c["previous"]);
        let person = person(&c["person"]);
        let rates: [Rates; 3] = serde_json::from_value(c["rates"].clone()).unwrap();
        let curves: Vec<Curve> = serde_json::from_value(c["curves"].clone()).unwrap();
        for curve in &curves {
            curve.validate().unwrap();
        }
        motives::decay_with_precision(
            &mut values,
            &mut previous,
            &person,
            c["hour"].as_i64().unwrap() as i16,
            &rates,
            &curves,
            precision,
        );
        assert_eq!(json!(values), row["original"]["values"], "decay {i}");
        assert_eq!(
            json!(previous),
            row["original"]["previous"],
            "decay previous {i}"
        );
        assert_eq!(row["original"]["error"], Value::Null);
    }
    for (i, row) in report["scores"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let curves: Vec<Curve> = serde_json::from_value(c["curves"].clone()).unwrap();
        let ads: [Advertisement; 16] = serde_json::from_value(c["advertisements"].clone()).unwrap();
        for (name, ads) in [("current", None), ("advertised", Some(&ads))] {
            let value =
                motives::score(&floats(&c["values"]), &person(&c["person"]), &curves, ads).unwrap();
            assert_eq!(
                json!(value.to_bits()),
                row[name]["bits"],
                "score {i} {name}: {value}"
            );
            assert_eq!(row[name]["error"], Value::Null);
        }
    }
    for (i, row) in report["changes"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let operands: [u8; 8] = serde_json::from_value(c["operands"].clone()).unwrap();
        let mut entries = c["initial"].clone();
        let error = if operands[3] & 1 != 0 {
            entries = json!([]);
            Value::Null
        } else {
            match Change::new(
                operands[2],
                i16::from_le_bytes([operands[4], operands[5]]),
                i16::from_le_bytes([operands[6], operands[7]]),
            ) {
                Ok(change) => {
                    entries.as_array_mut().unwrap().push(json!([
                        change.motive,
                        change.delta,
                        change.target
                    ]));
                    Value::Null
                }
                Err(_) => json!(26),
            }
        };
        assert_eq!(entries, row["original"]["entries"], "change {i}");
        assert_eq!(error, row["original"]["error"], "change {i}");
    }
    for (i, row) in report["applies"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut values = floats(&c["values"]);
        let changes: Vec<Change> = c["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Change {
                motive: v[0].as_u64().unwrap() as u8,
                delta: (v[1].as_f64().unwrap() as f32).to_bits(),
                target: (v[2].as_f64().unwrap() as f32).to_bits(),
            })
            .collect();
        motives::apply_changes(&mut values, &changes);
        assert_eq!(json!(values), row["original"]["values"], "apply {i}");
        assert_eq!(row["original"]["error"], Value::Null);
    }
    for (i, row) in report["tuning"].as_array().unwrap().iter().enumerate() {
        for profile in 0..3 {
            let input = &row["input"][profile];
            let rates =
                Rates::from_lookup(profile, |name| input[name].as_f64().map(|v| v as f32)).unwrap();
            for (name, value) in serde_json::to_value(rates).unwrap().as_object().unwrap() {
                let bits = if name.ends_with("_hour") {
                    value.as_i64().unwrap() as u32
                } else {
                    (value.as_f64().unwrap() as f32).to_bits()
                };
                assert_eq!(
                    json!(bits),
                    row["original"]["rates"][profile][name],
                    "tuning {i} {profile} {name}"
                );
            }
        }
    }
    for (i, row) in report["updates"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut values = floats(&c["values"]);
        let mut person = person(&c["person"]);
        person[27] = c["counter"].as_i64().unwrap() as i16;
        person[29] = c["decay_disabled"].as_i64().unwrap() as i16;
        let mut state = UpdateState {
            previous: floats(&c["previous"]),
            last_decay: c["last_decay"].as_i64().unwrap() as i32,
        };
        let curves: Vec<Curve> = serde_json::from_value(c["curves"].clone()).unwrap();
        let tuning = Tuning {
            update_precision: precision,
            rates: serde_json::from_value(c["rates"].clone()).unwrap(),
            adult_mood: curves.clone(),
            child_mood: curves,
            adult_scores: vec![],
            child_scores: vec![],
        };
        let changes: Vec<Change> = c["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Change {
                motive: v[0].as_u64().unwrap() as u8,
                delta: (v[1].as_f64().unwrap() as f32).to_bits(),
                target: (v[2].as_f64().unwrap() as f32).to_bits(),
            })
            .collect();
        if c["complete"] == true {
            state.finish(
                &mut values,
                &mut person,
                c["hidden"] != 0,
                c["tick"].as_i64().unwrap() as i32,
                c["hour"].as_i64().unwrap() as i16,
                &tuning,
                None,
                &changes,
            );
        }
        assert_eq!(json!(values), row["original"]["values"], "update {i}");
        assert_eq!(
            json!(state.previous),
            row["original"]["previous"],
            "update previous {i}"
        );
        assert_eq!(
            json!(state.last_decay),
            row["original"]["last_decay"],
            "update last {i}"
        );
        assert_eq!(
            json!(person[27]),
            row["original"]["counter"],
            "update counter {i}"
        );
        assert_eq!(row["original"]["error"], Value::Null);
    }
    for (i, row) in report["curves"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let curve = Curve::parse(
            0,
            c["text"].as_str().unwrap(),
            c["capacity"].as_u64().unwrap() as usize,
        )
        .unwrap();
        assert_eq!(json!(curve.points), row["original"]["points"], "curve {i}");
        let inverses: Vec<u32> = curve
            .points
            .windows(2)
            .map(|p| ((1.0 / (f64::from(p[1][0]) - f64::from(p[0][0]))) as f32).to_bits())
            .collect();
        assert_eq!(
            json!(inverses),
            row["original"]["inverses"],
            "curve inverse {i}"
        );
    }
    println!(
        "Original comparisons passed: decay {}, scores {}, changes {}, application {}",
        report["decay"].as_array().unwrap().len(),
        report["scores"].as_array().unwrap().len(),
        report["changes"].as_array().unwrap().len(),
        report["applies"].as_array().unwrap().len()
    );
}
