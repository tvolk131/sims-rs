//! Actual original-code supplied-process-word vectors. These are routine checks,
//! not original-versus-Rust completed gameplay tick credit.
use serde_json::{Value, json};
use sims_vm::{
    World,
    autonomy::{self, Candidate, Selection, Tuning},
    autonomy_constants::{Field, Owner},
};

fn supplied() -> World {
    let mut owner = Owner::default();
    owner.process.words = [
        Some(0.0000001f32.to_bits()),
        Some(0.0000001f32.to_bits()),
        Some(0.1f32.to_bits()),
        Some(0.3f32.to_bits()),
        Some(0.6f32.to_bits()),
        Some(0.01f32.to_bits()),
        Some(0.02f32.to_bits()),
        Some(0.03f32.to_bits()),
        Some(0.000001f32.to_bits()),
        Some(4),
        None,
        None,
    ];
    World {
        autonomy_constants: Some(owner),
        ..Default::default()
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_process_attenuation_reads_only_selected_word() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/process-tuning-consumers.json")).unwrap();
    for row in report["attenuation"].as_array().unwrap() {
        let c = &row["input"];
        let code = c["code"].as_u64().unwrap() as u32;
        let visitor = c["visitor"].as_bool().unwrap();
        let mut world = World::default();
        if (2..=4).contains(&code) {
            let mut owner = Owner::default();
            let field = (if visitor {
                [
                    Field::VisitorLow,
                    Field::VisitorModerate,
                    Field::VisitorHigh,
                ]
            } else {
                [
                    Field::LowAttenuation,
                    Field::ModerateAttenuation,
                    Field::HighAttenuation,
                ]
            })[(code - 2) as usize];
            let value = c[if visitor {
                "visitor_attenuation"
            } else {
                "family_attenuation"
            }][(code - 2) as usize]
                .as_f64()
                .unwrap() as f32;
            owner.process.words[field.index()] = Some(value.to_bits());
            world.autonomy_constants = Some(owner);
            let reads = row["process_reads"].as_array().unwrap();
            assert_eq!(reads.len(), 1);
            assert_eq!(reads[0]["address"], format!("{:#x}", field.address()));
        } else {
            assert!(row["process_reads"].as_array().unwrap().is_empty());
        }
        let before = world.clone();
        let value = autonomy::process_attenuation(
            &world,
            code,
            c["explicit"].as_f64().unwrap() as f32,
            visitor,
        )
        .unwrap();
        assert_eq!(json!(value.to_bits()), row["original"]["bits"], "{c}");
        assert_eq!(world, before);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_random_count_is_already_an_integer_word_not_an_fcns_float() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/process-tuning-consumers.json")).unwrap();
    for row in report["selection"].as_array().unwrap() {
        let c = &row["input"];
        let mut world = supplied();
        let owner = world.autonomy_constants.as_mut().unwrap();
        let raw = c["random_count"].as_u64().unwrap() as u32;
        owner.process.words[Field::RandomCount.index()] = Some(raw);
        owner.process.words[Field::SittingThreshold.index()] =
            Some((c["sitting_threshold"].as_f64().unwrap() as f32).to_bits());
        let tuning = Tuning::from_world(&world).unwrap();
        assert_eq!(tuning.random_count, raw as i32);
        let policy = Selection {
            posture: c["posture"].as_i64().unwrap() as i16,
            person18: c["person18"].as_i64().unwrap() as i16,
            visitor: c["visitor"].as_i64().unwrap() as i16,
            live_family: c["live_family"].as_bool().unwrap(),
            random_count: tuning.random_count,
            sitting_threshold: tuning.sitting_threshold,
        };
        let mut candidates: Vec<Candidate> =
            serde_json::from_value(c["candidates"].clone()).unwrap();
        let mut rng = c["rng"].as_u64().unwrap() as u32;
        let selected = autonomy::select(&mut candidates, &policy, &mut rng).unwrap();
        let original = &row["original"];
        assert_eq!(json!(candidates), original["ordered"], "{c}");
        assert_eq!(json!(selected.is_some()), original["selected"], "{c}");
        assert_eq!(json!(rng), original["rng"], "{c}");
        assert_eq!(
            json!(selected.as_ref().map_or(-5, |e| e.object)),
            original["target"],
            "{c}"
        );
        assert_eq!(
            json!(selected.as_ref().map_or(777, |e| e.menu as i16)),
            original["menu"],
            "{c}"
        );
    }
}

#[test]
fn unknown_process_fields_are_not_defaulted_or_inferred_from_loaded() {
    let mut world = supplied();
    assert_eq!(Tuning::from_world(&world).unwrap().random_count, 4);
    let owner = world.autonomy_constants.as_mut().unwrap();
    owner.process.loaded = Some(1);
    owner.process.words[Field::LowAttenuation.index()] = None;
    assert!(
        autonomy::process_attenuation(&world, 2, 0.7, false)
            .unwrap_err()
            .contains("LowAttenuation unavailable")
    );
    assert_eq!(
        autonomy::process_attenuation(&world, 3, 0.7, false)
            .unwrap()
            .to_bits(),
        0.3f32.to_bits()
    );
    assert!(Tuning::from_world(&world).is_err());
    world.autonomy_constants.as_mut().unwrap().process.words[Field::LowAttenuation.index()] =
        Some(f32::NAN.to_bits());
    assert!(
        autonomy::process_attenuation(&world, 2, 0.7, false)
            .unwrap_err()
            .contains("nonfinite")
    );
    world.autonomy_constants.as_mut().unwrap().process.words[Field::LowAttenuation.index()] =
        Some((-0.0f32).to_bits());
    assert_eq!(
        autonomy::process_attenuation(&world, 2, 0.7, false)
            .unwrap()
            .to_bits(),
        0x80000000
    );
    world.autonomy_constants = None;
    assert_eq!(
        autonomy::process_attenuation(&world, 0, 0.7, true).unwrap(),
        0.7
    );
    assert_eq!(
        autonomy::process_attenuation(&world, 1, 0.7, true).unwrap(),
        0.
    );
    assert_eq!(
        autonomy::process_attenuation(&world, 99, 0.7, true).unwrap(),
        0.02
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn actual_goth_baseline_process_words_do_not_require_earlier_fpu_or_fcns_inputs() {
    let owner: Owner =
        serde_json::from_str(include_str!("fixtures/actual-goth-process-baseline.json")).unwrap();
    owner.validate().unwrap();
    assert_eq!(owner.process.loaded, Some(1));
    assert!(owner.configuration.fp_control.is_none());
    assert!(owner.configuration.source.is_none());
    let world = World {
        autonomy_constants: Some(owner),
        ..Default::default()
    };
    let before = world.clone();
    let tuning = Tuning::from_world(&world).unwrap();
    assert_eq!(tuning.random_count, 4);
    assert_eq!(tuning.family_threshold.to_bits(), 0x33d6bf95);
    assert_eq!(tuning.visitor_threshold.to_bits(), 0x33d6bf95);
    assert_eq!(tuning.sitting_threshold.to_bits(), 0x358637bd);
    for (code, family, visitor) in [
        (2, 0x3dcccccdu32, 0x3c23d70au32),
        (3, 0x3e99999a, 0x3ca3d70a),
        (4, 0x3f19999a, 0x3cf5c28f),
    ] {
        assert_eq!(
            autonomy::process_attenuation(&world, code, 99., false)
                .unwrap()
                .to_bits(),
            family
        );
        assert_eq!(
            autonomy::process_attenuation(&world, code, 99., true)
                .unwrap()
                .to_bits(),
            visitor
        );
    }
    assert_eq!(world, before);
}
