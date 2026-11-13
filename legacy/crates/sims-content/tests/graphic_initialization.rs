use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::{
    Index,
    character_geometry::MeshPool,
    graphic_initialization::{Environment, Prepared},
};
use sims_sim::pose::{
    animator::Settings,
    graphic_frame,
    graphic_transform::Source,
    transform::{Terrain, View},
};
use sims_vm::Definition;

#[path = "../../sims-sim/tests/common/native_graphic_bound.rs"]
mod native_graphic_bound;

fn empty_index() -> Index {
    Index {
        root: Default::default(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    }
}
fn definition() -> Definition {
    Definition {
        source: "explicit fixture profile".into(),
        object: 1,
        guid: 1,
        slots: 0,
    }
}
fn settings() -> Settings {
    Settings {
        fade_enabled: true,
        fade_duration: 200,
        sampling_config: 0,
    }
}
fn words() -> Vec<i16> {
    let mut words = vec![0; 103];
    words[9] = 4;
    words[102] = 1;
    words
}

#[cfg(feature = "private-fixtures")]
#[test]
fn content_initializer_matches_composed_native_root_construction_and_restores_each_service()
-> Result<()> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../sims-sim/tests/fixtures/native-graphic-root-initialize.json"
    ))?;
    let index = empty_index();
    for (n, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let expected = &row["original"];
        let c = &input["transform"];
        let size = c["size"].as_u64().unwrap() as usize;
        let corners: [u8; 4] = serde_json::from_value(c["corners"].clone())?;
        let heights = (0..size)
            .flat_map(|x| {
                (0..size).map(move |y| corners.map(|v| v.wrapping_add((x * 3 + y * 5) as u8)))
            })
            .collect::<Vec<_>>();
        let source = Source {
            position: serde_json::from_value(c["position"].clone())?,
            facing: c["facing"].as_i64().unwrap() as i16,
            level: c["level"].as_i64().unwrap() as i32,
        };
        let view = View {
            rotation: c["rotation"].as_u64().unwrap() as u8,
            floor_vector: serde_json::from_value(
                c["floor_vectors"][c["zoom"].as_u64().unwrap() as usize].clone(),
            )?,
        };
        let mut results = vec![];
        for resume in [false, true] {
            let mut crt = input["crt_state"].as_u64().unwrap() as u32;
            let mut pool = MeshPool::default();
            let mut prepared = Prepared::begin(
                1,
                definition(),
                &words(),
                vec![
                    input["profile_name"].as_str().unwrap().into(),
                    "head-arrow,ROOT=white".into(),
                ],
                input["tick"].as_i64().unwrap() as i32,
                settings(),
            )?;
            let mut services = 0;
            while !prepared.complete() {
                if resume {
                    let saved = prepared.snapshot();
                    prepared = Prepared::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                    )?;
                    ensure!(
                        prepared.snapshot() == saved,
                        "case {n}: restored continuation changed"
                    );
                }
                prepared.advance(
                    &index,
                    &mut crt,
                    &mut pool,
                    Environment {
                        owner_appearance_index: input["index"].as_i64().unwrap() as i16,
                        source,
                        view,
                        terrain: Terrain {
                            size,
                            corners: &heights,
                        },
                        viewport: [-10, -20, 640, 480],
                    },
                )?;
                services += 1;
                ensure!(services <= 20);
            }
            let saved = prepared.snapshot();
            // Completion itself must remain resumable and idempotent.
            prepared = Prepared::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
            )?;
            ensure!(prepared.advance(
                &index,
                &mut crt,
                &mut pool,
                Environment {
                    owner_appearance_index: 0,
                    source,
                    view,
                    terrain: Terrain {
                        size,
                        corners: &heights
                    },
                    viewport: [0; 4]
                }
            )?);
            ensure!(prepared.snapshot() == saved);
            let saved_json = serde_json::to_value(&saved)?;
            for field in ["flags", "appearance_index", "selection"] {
                assert_eq!(
                    saved_json["state"][field], expected[field],
                    "case {n} {field}"
                );
            }
            assert_eq!(saved_json["engaged"], expected["engaged"]);
            assert_eq!(saved_json["callbacks"], json!({"1":1}));
            assert_eq!(
                saved_json["vectors"],
                json!({"generic":[0],"accessories":[]})
            );
            let ready = prepared.into_ready()?;
            assert_eq!(json!(crt), expected["bound"]["crt_state"]);
            native_graphic_bound::compare(&ready.bound, &expected["bound"], 3e-5);
            let mut frame: graphic_frame::State =
                serde_json::from_value(expected["frame"].clone())?;
            native_graphic_bound::matrix(ready.frame.matrix, frame.matrix, 3e-5);
            frame.matrix = ready.frame.matrix;
            assert_eq!(ready.frame, frame, "case {n}");
            assert!(ready.avatar.avatar.parts.is_empty());
            assert!(
                ready
                    .avatar
                    .records
                    .entries
                    .iter()
                    .all(|r| r.binding.is_none())
            );
            assert!(ready.geometry.vertices.is_empty());
            assert!(
                !ready.appearance.marker,
                "fresh appearance reload has no model to classify"
            );
            results.push((saved, crt, pool));
        }
        assert_eq!(
            results[0], results[1],
            "case {n}: save/resume changed initialization"
        );
    }
    Ok(())
}

#[test]
fn initializer_rejects_wrong_definition_or_incoherent_saved_owner() -> Result<()> {
    let profile = vec!["root".into(), "head-arrow,ROOT=white".into()];
    let mut person = words();
    person[9] = 2;
    assert!(Prepared::begin(1, definition(), &person, profile.clone(), 0, settings()).is_err());
    let prepared = Prepared::begin(1, definition(), &words(), profile, 0, settings())?;
    let saved = serde_json::to_value(prepared.snapshot())?;
    for (field, value) in [
        ("object", json!(0)),
        ("bound", json!({})),
        ("callbacks", json!({"2":2})),
    ] {
        let mut invalid = saved.clone();
        invalid[field] = value;
        if let Ok(invalid) = serde_json::from_value(invalid) {
            assert!(
                Prepared::restore(&empty_index(), invalid).is_err(),
                "{field}"
            );
        }
    }
    Ok(())
}

#[test]
fn missing_asset_retains_pending_bind_and_does_not_consume_rng_or_acknowledge_success() -> Result<()>
{
    let index = empty_index();
    let mut prepared = Prepared::begin(
        1,
        definition(),
        &words(),
        vec!["missing-rig".into(), "appearance".into()],
        0,
        settings(),
    )?;
    let mut crt = 1;
    let mut pool = MeshPool::default();
    let environment = || Environment {
        owner_appearance_index: 0,
        source: Source {
            position: [-16, -16],
            facing: 0,
            level: 1,
        },
        view: View {
            rotation: 0,
            floor_vector: [0.; 3],
        },
        terrain: Terrain {
            size: 2,
            corners: &[[0; 4]; 4],
        },
        viewport: [0; 4],
    };
    let mut failed = false;
    for _ in 0..20 {
        if prepared
            .advance(&index, &mut crt, &mut pool, environment())
            .is_err()
        {
            failed = true;
            break;
        }
    }
    ensure!(failed && !prepared.complete());
    let saved = prepared.snapshot();
    let state = serde_json::to_value(&saved)?;
    assert_eq!(state["state"]["model"], json!(1));
    assert_eq!(state["callbacks"], json!({"1":null}));
    assert!(state["bound"].is_null() && state["update"]["bind_status"].is_null());
    assert_eq!(crt, 1);
    prepared = Prepared::restore(&index, saved.clone())?;
    assert!(
        prepared
            .advance(&index, &mut crt, &mut pool, environment())
            .is_err()
    );
    assert_eq!(prepared.snapshot(), saved);
    assert_eq!(crt, 1);
    Ok(())
}
