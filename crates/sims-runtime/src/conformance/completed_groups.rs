//! Actual native multipart coordinates and links through the gameplay traversal.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_groups::{self, Input as Group};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use sims_vm::{ObjectId, multitile};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-groups-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    objects: BTreeMap<ObjectId, u32>,
    groups: Vec<Group>,
    unqualified: Vec<String>,
    scope: String,
    provenance: Value,
}
fn evaluate(i: Input) -> Result<Value> {
    ensure!(
        i.schema == 1
            && i.contract == CONTRACT
            && i.tick == 88448
            && i.native_parity_ticks == 0
            && i.unqualified.len() >= 3
            && !i.scope.is_empty(),
        "retained group component profile differs"
    );
    ensure!(
        i.provenance["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && i.provenance["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "retained group source epoch differs"
    );
    ensure!(
        i.objects.len() == 344 && i.groups.len() == 142,
        "original group coverage differs"
    );
    let groups = completed_groups::import(&i.groups, &i.objects)?;
    let mut chains = vec![];
    for (&id, part) in &groups {
        if part.head.is_none() {
            chains.push(multitile::retained_chain(&groups, id).map_err(anyhow::Error::msg)?);
        }
    }
    let binary = postcard::to_allocvec(&groups)?;
    let restored: BTreeMap<ObjectId, multitile::Part> = postcard::from_bytes(&binary)?;
    ensure!(
        restored == groups && postcard::to_allocvec(&restored)? == binary,
        "retained group binary roundtrip differs"
    );
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":i.tick,"native_parity_ticks":0,"objects":i.objects.len(),"parts":groups.len(),"chains":chains,"origin_rotation_links_equal":true,"cold_binary_roundtrip_equal":true,"saved_sha256":sims_fixtures::digest(&binary),"unqualified":i.unqualified,"provenance":i.provenance}),
    )
}
pub fn run(store: &Store, m: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "group fixture contract differs"
    );
    let report = evaluate(serde_json::from_slice(
        &store.read_bytes(&m.source.blob, 16 << 20)?,
    )?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-groups.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:142,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("142 native multi-tile parts match Rust origin/rotation/chain semantics and binary roundtrip; constructor permissions and complete VM/World remain unqualified".into()),artifacts:vec![PathBuf::from("completed-groups.json")]})
}
