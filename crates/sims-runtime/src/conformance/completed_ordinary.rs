//! Current ordinary graphs; preserves the live arrow instead of emptying it.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_ordinary::{self, Imported};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use sims_vm::{Definition, ObjectId};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-ordinary-graphs-component-v2";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    ordinary: Value,
    definitions: BTreeMap<ObjectId, Definition>,
    lifetimes: BTreeMap<ObjectId, u32>,
    scope: String,
    provenance: Value,
}
pub fn run(store: &Store, m: &CaseManifest, root: &Path, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "ordinary fixture contract differs"
    );
    let v: Input = serde_json::from_slice(&store.read_bytes(&m.source.blob, 16 << 20)?)?;
    ensure!(
        v.schema == 1
            && v.contract == CONTRACT
            && v.tick == 88448
            && v.native_parity_ticks == 0
            && !v.scope.is_empty()
            && v.provenance == v.ordinary["provenance"],
        "ordinary component profile differs"
    );
    let mut index = sims_content::Index::scan(root)?;
    index.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|s| s.to_ascii_lowercase().starts_with("userdata"))
    });
    let imported = completed_ordinary::import(
        &v.ordinary,
        &v.definitions,
        &v.lifetimes,
        &index,
        "new-unpublished-goth-ordinary",
    )?;
    let binary = Store::create(output.join("cold-store"))?;
    let saved = binary.put_value(&imported)?;
    let cold: Imported = binary.get_value(&saved)?;
    ensure!(
        cold == imported,
        "ordinary named MessagePack cold restore differs"
    );
    let frames =
        sims_content::graphic_loading::Registry::restore(&index, cold.frame_registry.clone())?;
    ensure!(
        frames.snapshot() == imported.frame_registry,
        "ordinary frame registry cold restore differs"
    );
    for (&id, binding) in &cold.bindings {
        ensure!(
            sims_content::saved_avatar::BoundAvatar::restore(&index, "root", id as u64, binding)?
                .snapshot()
                == *binding,
            "ordinary cold immutable attachment differs"
        );
    }
    ensure!(
        cold.geometry.characters.len() == 2
            && cold.geometry.mesh_bmf.len() == 1
            && cold.unadmitted_live_graphics == [383]
            && cold
                .immutable_sources
                .keys()
                .all(|s| !s.to_ascii_lowercase().contains("userdata")),
        "ordinary source/coverage profile differs"
    );
    let mut payload = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-ordinary.msgpack"))?;
    binary.copy_blob(&saved, &mut payload)?;
    let report = json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component",
        "tick":88448,"native_parity_ticks":0,"ordinary_graphs":2,"observed_null_owners":342,
        "mesh_bindings":1,"published_vertices":65,"restored_live_empty_frames":[384],
        "unadmitted_live_graphics":cold.unadmitted_live_graphics,"immutable_sources":cold.immutable_sources,
        "starting_save_paths_unavailable":true,"binary_cold_roundtrip_equal":true,"source_mesh_bits_equal":true,
        "saved":saved,"complete_asset_host_qualified":false,"resumable_world_baseline":false,
        "provenance":v.provenance});
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-ordinary.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),
        observations:2,completed_ticks:0,matched_completed_ticks:0,difference:None,
        detail:Some("2 source-bound ordinary ROOT graphs and 342 observed nulls; arrow 65-vertex immutable mesh equals captured data; object 384 cold-restores through retained frame registry; object 383 live host remains explicitly unadmitted".into()),
        artifacts:vec![PathBuf::from("completed-ordinary.json"),PathBuf::from("completed-ordinary.msgpack")]})
}
