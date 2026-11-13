use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_sim::pose::{layer::Layer, sampling::BoneInput, update::Update};

fn pose(bone: &BoneInput, expected: &Value, context: &str) -> Result<()> {
    let t: [f32; 3] = serde_json::from_value(expected["translation"].clone())?;
    let r: [f32; 4] = serde_json::from_value(expected["rotation"].clone())?;
    for (a, b) in bone.local.translation.into_iter().zip(t) {
        ensure!((a - b).abs() < 3e-5, "translation {context}: {a} vs {b}");
    }
    for (a, b) in bone.local.rotation.into_iter().zip(r) {
        ensure!((a - b).abs() < 1e-6, "rotation {context}: {a} vs {b}");
    }
    Ok(())
}
#[test]
#[ignore = "requires private original animations and native full-layer update report"]
fn original_clips_match_events_and_pose_before_callbacks_and_after_each_update() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-layer-updates.json"),
    )?)?;
    ensure!(rows.len() == 8);
    let mut program = sims_vm::Program::default();
    let mut callbacks = 0;
    for row in &rows {
        let mut layer: Layer = serde_json::from_value(row["input"]["layer"].clone())?;
        let mut bones: Vec<BoneInput> = serde_json::from_value(row["input"]["bones"].clone())?;
        sims_content::assets::load_named_animations(
            &index,
            &mut program,
            &[layer.name.clone()].into(),
        )?;
        let asset = program.animations.get(&layer.name).context("animation")?;
        let direction = layer.clock.speed;
        for (i, step) in row["steps"].as_array().context("steps")?.iter().enumerate() {
            let context = format!("{} speed {direction} step {i}", layer.name);
            let now = layer
                .clock
                .current
                .wrapping_add(step["elapsed"].as_i64().unwrap() as i32);
            let mut update =
                Update::begin(&mut layer, now, true, true).map_err(anyhow::Error::msg)?;
            let expected = &step["original"];
            let mut event_count = 0;
            while let Some(event) = update
                .next(&mut layer, &asset.clip, &asset.samples, &mut bones)
                .map_err(anyhow::Error::msg)?
            {
                let original = &expected["events"][event_count];
                ensure!(
                    original["binding"] == event.binding && original["event"] == event.event,
                    "event identity {context}"
                );
                ensure!(
                    event.phase.to_bits() == (original["phase"].as_f64().unwrap() as f32).to_bits(),
                    "callback phase {context}"
                );
                pose(
                    &bones[event.bone],
                    original,
                    &format!("callback {event_count} {context}"),
                )?;
                (layer, update, bones) =
                    serde_json::from_slice(&serde_json::to_vec(&(&layer, &update, &bones))?)?;
                ensure!(
                    update
                        .next(&mut layer, &asset.clip, &asset.samples, &mut bones)
                        .map_err(anyhow::Error::msg)?
                        == Some(event),
                    "pending callback changed {context}"
                );
                update.acknowledge().map_err(anyhow::Error::msg)?;
                callbacks += 1;
                event_count += 1;
            }
            ensure!(
                event_count == expected["events"].as_array().unwrap().len(),
                "event count {context}"
            );
            ensure!(
                layer.clock == serde_json::from_value(expected["clock"].clone())?,
                "clock {context}"
            );
            for (binding, cursor) in layer
                .bindings
                .iter()
                .zip(expected["cursors"].as_array().unwrap())
            {
                ensure!(
                    binding.event_cursor == cursor.as_i64().map(|v| v as i32),
                    "cursor {context}"
                );
            }
            for (j, (bone, original)) in bones
                .iter()
                .zip(expected["bones"].as_array().unwrap())
                .enumerate()
            {
                pose(bone, original, &format!("{context} bone {j}"))?;
            }
            (layer, bones) = serde_json::from_slice(&serde_json::to_vec(&(&layer, &bones))?)?;
        }
    }
    ensure!(callbacks >= 8);
    println!("Compared 800 original layer updates and {callbacks} property callbacks");
    Ok(())
}
