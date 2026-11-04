//! Publish source-bound ordinary graphs only after validating their component.
use anyhow::{Context, Result, ensure};
use sims_fixtures::{Artifact, ArtifactFormat, CaseManifest, Compression, Coverage, Origin, Store};
use sims_runtime::{
    conformance::completed_ordinary::CONTRACT, original_session::completed_ordinary,
};
use std::{collections::BTreeMap, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 4,
        "usage: completed_ordinary_graphs STORE NEW_CASE INPUT IMMUTABLE_CONTENT"
    );
    let raw = std::fs::read(&a[2])?;
    ensure!(raw.len() <= 16 << 20, "ordinary input size exceeds profile");
    let input: serde_json::Value = serde_json::from_slice(&raw)?;
    ensure!(
        input["contract"] == CONTRACT && input["native_parity_ticks"] == 0,
        "ordinary contract differs"
    );
    let definitions: BTreeMap<sims_vm::ObjectId, sims_vm::Definition> =
        serde_json::from_value(input["definitions"].clone())?;
    let lifetimes = serde_json::from_value(input["lifetimes"].clone())?;
    let mut index = sims_content::Index::scan(Path::new(&a[3]))?;
    index.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|s| s.to_ascii_lowercase().starts_with("userdata"))
    });
    let imported = completed_ordinary::import(
        input.get("ordinary").context("ordinary source absent")?,
        &definitions,
        &lifetimes,
        &index,
        "new-unpublished-goth-ordinary",
    )?;
    ensure!(
        imported.geometry.characters.len() == 2 && imported.unadmitted_live_graphics == [383],
        "ordinary component coverage differs"
    );
    let store = Store::open(&a[0])?;
    let blob = store.put_bytes(&raw, Compression::Gzip)?;
    let limitations = vec![
        "live arrow383 graphic/appearance/frame host remains unadmitted".into(),
        "complete asset host, World, Simulation and Session".into(),
        "next natural original completed tick and strict comparison".into(),
    ];
    let mut case = CaseManifest::minimal_import(
        &a[1],
        "Goth88448 ordinary ROOT graphs and selected-person arrow",
        Artifact::self_contained(blob.clone(), ArtifactFormat::Json, 1),
        limitations.clone(),
    );
    case.provenance.origin = Origin::Observed;
    case.provenance.source_locator =
        Some("native-goth-completed-physical-baseline-20261027-v1:tick88448".into());
    case.provenance.commands.push(vec![
        "python3".into(),
        "-B".into(),
        "scripts/research/project_completed_ordinary.py".into(),
    ]);
    case.provenance.notes=vec!["Exact current root/appearance/drawable/notification graph and read receipts; no native update or RNG executed.".into(),
        "Both immutable meshes/records and binary cold component are testable; only384is in the old retained frame adapter. No graph is emptied for admission.".into()];
    case.coverage = Coverage::Component {
        scope: CONTRACT.into(),
        limitations,
    };
    store.publish_case(&case)?;
    println!(
        "{}",
        serde_json::json!({"case":a[1],"source":blob,"native_parity_ticks":0})
    );
    Ok(())
}
