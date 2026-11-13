use super::super::Documents;
use crate::{Assets, Texture};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sims_content::Index;
use std::{collections::BTreeSet, fmt::Write, path::PathBuf, sync::Arc, time::Instant};

#[derive(Deserialize)]
struct Selectors {
    before: BTreeSet<(String, u16)>,
    after: BTreeSet<(String, u16)>,
}

struct Fingerprint(Sha256);

impl Write for Fingerprint {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.0.update(value.as_bytes());
        Ok(())
    }
}

fn fingerprint(assets: &Assets) -> Result<String> {
    let mut fingerprint = Fingerprint(Sha256::new());
    write!(&mut fingerprint, "{assets:?}")?;
    Ok(format!("{:x}", fingerprint.0.finalize()))
}

fn load(
    index: &Index,
    definitions: &BTreeSet<(String, u16)>,
    documents: &mut Documents,
) -> Result<Assets> {
    Assets::load_objects_profile_decoded(
        index,
        &definitions
            .iter()
            .map(|(source, identifier)| (source.as_str(), *identifier))
            .collect::<Vec<_>>(),
        Some(&BTreeSet::new()),
        documents,
    )
}

fn address(texture: &Texture) -> (usize, usize) {
    (
        texture.rgba.as_ptr() as usize,
        texture.depth.as_ptr() as usize,
    )
}

#[test]
#[ignore = "requires installed assets and private before/after graphic selectors"]
fn retained_renderer_documents_preserve_complete_assets_across_resource_growth() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("SIMS_RENDER_TEST_ASSET_ROOT").context("asset root is required")?,
    );
    let selector_path = PathBuf::from(
        std::env::var_os("SIMS_RENDER_TEST_GROWTH_SELECTORS").context("selectors are required")?,
    );
    ensure!(selector_path.metadata()?.len() <= 1024 * 1024);
    let selectors: Selectors = serde_json::from_slice(&std::fs::read(selector_path)?)?;
    let expanded: BTreeSet<_> = selectors.before.union(&selectors.after).cloned().collect();
    ensure!(!selectors.before.is_empty());
    ensure!(expanded.len() > selectors.before.len());
    let index = Index::scan(&root)?;
    let mut documents = Documents::default();
    let retained_before = load(&index, &selectors.before, &mut documents)?;
    let before_fingerprint = fingerprint(&retained_before)?;
    let independent_before = load(&index, &selectors.before, &mut Documents::default())?;
    assert_eq!(before_fingerprint, fingerprint(&independent_before)?);
    drop(independent_before);
    let started = Instant::now();
    let expanded_assets = load(&index, &expanded, &mut documents)?;
    let growth_microseconds = started.elapsed().as_micros();
    let expanded_fingerprint = fingerprint(&expanded_assets)?;
    let independent_after = load(&index, &expanded, &mut Documents::default())?;
    assert_eq!(expanded_fingerprint, fingerprint(&independent_after)?);
    assert_eq!(before_fingerprint, fingerprint(&retained_before)?);
    let expanded_addresses: BTreeSet<_> = expanded_assets.textures.iter().map(address).collect();
    let before_object_textures: BTreeSet<_> = retained_before
        .sprites
        .values()
        .flatten()
        .map(|sprite| sprite.texture)
        .collect();
    ensure!(!before_object_textures.is_empty());
    for identifier in &before_object_textures {
        let texture = &retained_before.textures[*identifier];
        assert!(expanded_addresses.contains(&address(texture)));
        let shared = expanded_assets
            .textures
            .iter()
            .find(|candidate| address(candidate) == address(texture))
            .unwrap();
        assert!(Arc::ptr_eq(&texture.rgba, &shared.rgba));
        assert!(Arc::ptr_eq(&texture.depth, &shared.depth));
    }
    let repeated_after = load(&index, &expanded, &mut documents)?;
    assert_eq!(expanded_fingerprint, fingerprint(&repeated_after)?);
    println!(
        "{}",
        json!({
            "scope": "Complete Assets equality and retained payload identity; no gameplay or native qualification",
            "before_definitions": selectors.before.len(),
            "expanded_definitions": expanded.len(),
            "before_textures": retained_before.textures.len(),
            "expanded_textures": expanded_assets.textures.len(),
            "shared_before_object_textures": before_object_textures.len(),
            "before_complete_assets_sha256": before_fingerprint,
            "expanded_complete_assets_sha256": expanded_fingerprint,
            "growth_microseconds": growth_microseconds,
        })
    );
    Ok(())
}
