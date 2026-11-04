use serde_json::{Value, json};
use sims_vm::look::{self, Preparation};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_turn_preparation_and_completion_vectors() {
    let report: Value = serde_json::from_str(include_str!("fixtures/look.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let actor = serde_json::from_value(c["actor"].clone()).unwrap();
        let target = match c["target_id"].as_i64().unwrap() {
            1 => Some(actor),
            2 => Some(serde_json::from_value(c["target"].clone()).unwrap()),
            _ => None,
        };
        let start = look::prepare(
            c["mode"].as_i64().unwrap() as i16,
            c["continuation"].as_u64().unwrap() as u32,
            c["posture"].as_i64().unwrap() as i16,
            actor,
            target,
            c["camera"].as_u64().unwrap() as u8,
        );
        if let Some(error) = n["error"].as_u64() {
            assert!(start.unwrap_err().contains(&format!("0x{error:x}")));
            continue;
        }
        let start = start.unwrap();
        let began = matches!(start, Preparation::Start { .. });
        let (returned, finished) = if start == Preparation::Declined {
            (0, 0)
        } else {
            (
                look::complete(c["status"].as_i64().unwrap() as i32, c["finish"] == 1),
                i32::from(c["status"] != 2),
            )
        };
        assert_eq!(json!(returned), n["returned"], "{c}");
        assert_eq!(json!(finished), n["finished"]);
        assert_eq!(json!(u32::from(began)), n["started"]);
        assert_eq!(
            json!(if began {
                1
            } else {
                c["continuation"].as_u64().unwrap()
            }),
            n["continuation"]
        );
        let (path, data9) = if let Preparation::Start { path, clear_data9 } = start {
            (path.to_vec(), if clear_data9 { 0 } else { 99 })
        } else {
            (vec![], 99)
        };
        assert_eq!(json!(path), n["path"]);
        assert_eq!(json!(data9), n["data9"]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_standing_turn_clip_and_heading_correction_vectors() {
    let report: Value = serde_json::from_str(include_str!("fixtures/look.json")).unwrap();
    for row in report["turns"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let heading = c["heading"].as_f64().unwrap() as f32;
        let delta = c["delta"].as_f64().unwrap() as f32;
        let turn = look::standing_turn(heading, delta).unwrap();
        assert_eq!(json!(turn.clip), n["clip"]);
        assert_eq!(json!(turn.nominal_turn.to_bits()), n["span_bits"]);
        assert_eq!(json!(turn.base_heading.to_bits()), n["base_heading_bits"]);
        assert_eq!(json!(heading.to_bits()), n["from_heading_bits"]);
        assert_eq!(json!((heading + delta).to_bits()), n["target_heading_bits"]);
        assert_eq!(n["state"], 4);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_animator_clock_clip_progress_pose_and_target_vectors() {
    let report: Value = serde_json::from_str(include_str!("fixtures/turn-timing.json")).unwrap();
    for r in report["clocks"].as_array().unwrap() {
        let c = &r["input"];
        assert_eq!(
            look::animation_time(c["tick"].as_i64().unwrap() as i32, c["alternate"] == 1) as u32,
            r["original"].as_u64().unwrap() as u32
        );
    }
    for r in report["progress"].as_array().unwrap() {
        let c = &r["input"];
        let f = |k| c[k].as_f64().unwrap() as f32;
        let (p, ended) = look::advance_progress(
            f("progress"),
            c["time"].as_i64().unwrap() as i32 - c["previous_time"].as_i64().unwrap() as i32,
            f("duration"),
        );
        assert_eq!(json!(p.to_bits()), r["original"]["progress_bits"], "{c}");
        assert_eq!(json!(u32::from(ended)), r["original"]["ended"], "{c}");
    }
    for r in report["poses"].as_array().unwrap() {
        let c = &r["input"];
        let f = |k| c[k].as_f64().unwrap() as f32;
        let heading = look::pose_heading(f("from"), f("base"), f("target"), f("progress"));
        assert_eq!(
            json!(heading.to_bits()),
            r["original"]["heading_bits"],
            "{c}"
        );
        assert_eq!(
            json!(look::quantized_heading(heading)),
            r["original"]["direction"],
            "{c}"
        );
    }
    for r in report["targets"].as_array().unwrap() {
        let c = &r["input"];
        assert_eq!(
            json!(
                look::target_heading([
                    c["dx"].as_i64().unwrap() as i32,
                    c["dy"].as_i64().unwrap() as i32
                ])
                .to_bits()
            ),
            r["original"],
            "{c}"
        );
    }
}
