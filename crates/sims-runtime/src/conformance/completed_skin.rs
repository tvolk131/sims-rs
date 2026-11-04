//! Separate native skin/property pass closure and exact observed rig recipes.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_skin::{self, Qualified};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
pub const CONTRACT: &str = "completed-native-skin-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    skeleton_input: Value,
    proof: completed_skin::Input,
    scope: String,
    unqualified: Vec<String>,
    provenance: Value,
}
pub(super) fn import_parts(v: &Value) -> Result<BTreeMap<i16, Qualified>> {
    evaluate(serde_json::from_value(v.clone())?)?;
    let i:Input=serde_json::from_value(v.clone())?;
    completed_skin::import(super::completed_skeletons::import_parts(&i.skeleton_input)?,&i.proof,i.proof.image_sha256.as_str())
}
fn evaluate(i: Input) -> Result<Value> {
    ensure!(
        i.schema == 1
            && i.contract == CONTRACT
            && i.tick == 88448
            && i.native_parity_ticks == 0
            && !i.scope.is_empty()
            && i.unqualified.len() >= 5,
        "native skin component scope/profile differs"
    );
    ensure!(
        i.provenance["image_sha256"] == i.proof.image_sha256
            && i.provenance["observation_sha256"] == i.proof.observation_sha256
            && i.skeleton_input["provenance"]["image_sha256"] == i.provenance["image_sha256"]
            && i.skeleton_input["provenance"]["observation_sha256"]
                == i.provenance["observation_sha256"],
        "skin/skeleton source epoch differs"
    );
    let parts = super::completed_skeletons::import_parts(&i.skeleton_input)?;
    let skins = completed_skin::import(parts, &i.proof, i.proof.image_sha256.as_str())?;
    ensure!(
        skins.keys().copied().collect::<Vec<_>>() == [16, 18, 21]
            && skins.values().all(|s| s.instance.skin.bones.len() == 29),
        "retained skin coverage differs"
    );
    let binary = postcard::to_allocvec(&skins)?;
    let cold: BTreeMap<i16, Qualified> = postcard::from_bytes(&binary)?;
    ensure!(
        skins == cold && postcard::to_allocvec(&cold)? == binary,
        "retained skin/rig binary cold roundtrip differs"
    );
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":i.tick,"native_parity_ticks":0,"skin_instances":3,"rigs":3,"bones":87,"separate_synchronous_property_closure":true,"registered_callback_is_not_pending_work":true,"native_events_queues_not_declared_absent":true,"source_anchor_recipes_checked":true,"binary_cold_roundtrip_equal":true,"saved_sha256":sims_fixtures::digest(&binary),"complete_character_qualified":false,"unqualified":i.unqualified,"provenance":i.provenance}),
    )
}
pub fn run(store: &Store, m: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "native skin fixture contract differs"
    );
    let report = evaluate(serde_json::from_slice(
        &store.read_bytes(&m.source.blob, 16 << 20)?,
    )?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-skin.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:3,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("Three retained skin Instances and Rigs compose only after separate synchronous source skin/property closure, actual registered callback, head flags and exact anchor/lookup recipe validation; cold roundtrip passes, full Character/World/Session and gameplay ticks remain unqualified".into()),artifacts:vec![PathBuf::from("completed-skin.json")]})
}
