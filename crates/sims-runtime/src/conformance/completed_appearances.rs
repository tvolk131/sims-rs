//! Actual current appearance bindings, with immutable-only reconstruction.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_appearances::{self, Imported};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-appearance-bindings-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    prototypes_input: Value,
    appearance_code: Value,
    provenance: Value,
    scope: String,
}
fn evaluate(v: Input, root: &Path) -> Result<Value> {
    ensure!(
        v.schema == 1
            && v.contract == CONTRACT
            && v.tick == 88448
            && v.native_parity_ticks == 0
            && !v.scope.is_empty()
            && v.provenance == v.prototypes_input["provenance"],
        "appearance binding component profile differs"
    );
    let mut index = sims_content::Index::scan(root)?;
    index
        .resources
        .retain(|r| !r.locator.split('/').any(|s| s.starts_with("UserData")));
    let before = super::completed_prototypes::import_parts(&v.prototypes_input)?;
    let before_bytes = postcard::to_allocvec(&before)?;
    let imported = completed_appearances::import(
        &v.prototypes_input["resident_input"]["resident_input"],
        &v.appearance_code,
        before,
        &index,
    )?;
    ensure!(
        postcard::to_allocvec(&imported.characters)? == before_bytes,
        "appearance reconstruction mutated retained characters/RNG-independent state"
    );
    let saved = postcard::to_allocvec(&imported)?;
    let cold: Imported = postcard::from_bytes(&saved)?;
    ensure!(
        cold == imported && postcard::to_allocvec(&cold)? == saved,
        "appearance binding cold restore differs"
    );
    for (&id, a) in &cold.saved {
        let avatar = sims_content::saved_avatar::BoundAvatar::restore(
            &index,
            &a.skeleton,
            id as u64,
            &a.binding,
        )?;
        ensure!(
            avatar.snapshot() == a.binding,
            "cold appearance attachment identities differ"
        );
    }
    ensure!(
        cold.immutable_sources
            .keys()
            .all(|s| !s.contains("UserData")),
        "appearance source used original starting saves"
    );
    let meshes = cold
        .saved
        .values()
        .map(|a| a.geometry.state.bindings.len())
        .sum::<usize>();
    ensure!(
        cold.saved.len() == 3 && meshes == 13,
        "appearance binding coverage differs"
    );
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":88448,"native_parity_ticks":0,"appearance_people":3,"mesh_bindings":meshes,"immutable_sources":cold.immutable_sources,"starting_save_paths_unavailable":true,"source_mesh_bits_equal":true,"retained_characters_unchanged":true,"binary_cold_roundtrip_equal":true,"saved_sha256":sims_fixtures::digest(&saved),"complete_asset_host_qualified":false,"resumable_world_baseline":false,"unqualified":cold.characters.characters.demands,"provenance":v.provenance}),
    )
}
pub fn run(store: &Store, m: &CaseManifest, root: &Path, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "appearance binding fixture contract differs"
    );
    let report = evaluate(
        serde_json::from_slice(&store.read_bytes(&m.source.blob, 16 << 20)?)?,
        root,
    )?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-appearances.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:13,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("3current appearance record/selection owners restore immutable avatars with13mesh bindings equal to captured resident meshes; original starting-save paths unavailable, retained characters unchanged; full asset-host/process/World admission remains unqualified".into()),artifacts:vec![PathBuf::from("completed-appearances.json")]})
}
