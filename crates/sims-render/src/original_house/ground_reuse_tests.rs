use super::*;
use crate::Texture;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fmt::Write, path::PathBuf, sync::Arc, time::Instant};

struct Fingerprint(Sha256);

impl Write for Fingerprint {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.0.update(value.as_bytes());
        Ok(())
    }
}

fn fingerprint(value: &impl std::fmt::Debug) -> Result<String> {
    let mut fingerprint = Fingerprint(Sha256::new());
    write!(&mut fingerprint, "{value:?}")?;
    Ok(format!("{:x}", fingerprint.0.finalize()))
}

fn restore(index: &sims_content::Index, variable: &str) -> Result<Session> {
    let path = PathBuf::from(std::env::var_os(variable).context("checkpoint path is required")?);
    ensure!(path.metadata()?.len() <= 64 * 1024 * 1024);
    let saved: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    Session::restore(index, serde_json::from_value(saved["session"].clone())?)
}

fn ground_texture(renderer: &OriginalHouse) -> Texture {
    let assets = renderer.assets.as_ref().unwrap();
    assets.textures[assets.ground.as_ref().unwrap().texture].clone()
}

#[test]
#[ignore = "requires installed assets and private before/after resource-growth checkpoints"]
fn retained_ground_pixels_preserve_complete_assets_scenes_and_session() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("SIMS_RENDER_TEST_ASSET_ROOT").context("asset root is required")?,
    );
    let index = sims_content::Index::scan(&root)?;
    let before = restore(&index, "SIMS_RENDER_TEST_BEFORE_SAVE")?;
    let after = restore(&index, "SIMS_RENDER_TEST_AFTER_SAVE")?;
    let before_definitions = OriginalHouse::live_definitions(&before);
    let after_definitions = OriginalHouse::live_definitions(&after);
    ensure!(before_definitions.is_subset(&after_definitions));
    ensure!(before_definitions.len() < after_definitions.len());
    let before_state = fingerprint(&before.save_managed()?)?;
    let after_state = fingerprint(&after.save_managed()?)?;
    let before_camera = OriginalHouse::native_camera(&before)?;
    let after_camera = OriginalHouse::native_camera(&after)?;
    let mut retained = OriginalHouse::default();
    let mut independent = OriginalHouse::default();
    let retained_before = retained.scene(&before, before_camera)?;
    let independent_before = independent.scene(&before, before_camera)?;
    let before_scene_fingerprint = fingerprint(&retained_before)?;
    assert_eq!(before_scene_fingerprint, fingerprint(&independent_before)?);
    assert_eq!(
        fingerprint(&retained.assets)?,
        fingerprint(&independent.assets)?
    );
    let before_ground = ground_texture(&retained);
    drop(independent_before);
    independent.ground_pixels = crate::grass::Cache::default();
    let started = Instant::now();
    let retained_after = retained.scene(&after, after_camera)?;
    let retained_growth_microseconds = started.elapsed().as_micros();
    let started = Instant::now();
    let independent_after = independent.scene(&after, after_camera)?;
    let cold_growth_microseconds = started.elapsed().as_micros();
    let after_scene_fingerprint = fingerprint(&retained_after)?;
    let after_assets_fingerprint = fingerprint(&retained.assets)?;
    assert_eq!(after_scene_fingerprint, fingerprint(&independent_after)?);
    assert_eq!(after_assets_fingerprint, fingerprint(&independent.assets)?);
    assert_eq!(before_scene_fingerprint, fingerprint(&retained_before)?);
    let after_ground = ground_texture(&retained);
    let input_unchanged = before.house.width == after.house.width
        && before.house.height == after.house.height
        && before.house.arrays.get(&6) == after.house.arrays.get(&6);
    assert_eq!(
        Arc::ptr_eq(&before_ground.rgba, &after_ground.rgba),
        input_unchanged
    );
    assert_eq!(
        Arc::ptr_eq(&before_ground.depth, &after_ground.depth),
        input_unchanged
    );
    retained.assets = None;
    let repeated_after = retained.scene(&after, after_camera)?;
    assert_eq!(after_scene_fingerprint, fingerprint(&repeated_after)?);
    assert_eq!(after_assets_fingerprint, fingerprint(&retained.assets)?);
    let repeated_ground = ground_texture(&retained);
    assert!(Arc::ptr_eq(&after_ground.rgba, &repeated_ground.rgba));
    assert!(Arc::ptr_eq(&after_ground.depth, &repeated_ground.depth));
    assert_eq!(before_state, fingerprint(&before.save_managed()?)?);
    assert_eq!(after_state, fingerprint(&after.save_managed()?)?);
    println!(
        "{}",
        json!({
            "scope": "Restored-state renderer differential including full ground/walls/floors, scenes and unchanged sessions; no gameplay or native qualification",
            "before_definitions": before_definitions.len(),
            "after_definitions": after_definitions.len(),
            "ground_input_unchanged": input_unchanged,
            "before_complete_scene_sha256": before_scene_fingerprint,
            "after_complete_scene_sha256": after_scene_fingerprint,
            "after_complete_assets_sha256": after_assets_fingerprint,
            "before_session_sha256": before_state,
            "after_session_sha256": after_state,
            "retained_growth_microseconds": retained_growth_microseconds,
            "cold_growth_microseconds": cold_growth_microseconds,
        })
    );
    Ok(())
}
