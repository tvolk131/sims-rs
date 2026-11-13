//! Native bone/hierarchy/cache parts; no Live/pass/Character/VM admission.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_skeletons::{self, Imported};
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
pub const CONTRACT: &str = "completed-native-skeleton-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    rows: Vec<completed_skeletons::Input>,
    animation_input: Value,
    scope: String,
    unqualified: Vec<String>,
    provenance: Value,
}
pub(super) fn import_parts(v: &Value) -> Result<BTreeMap<i16, Imported>> {
    evaluate(serde_json::from_value(v.clone())?)?;
    let i: Input = serde_json::from_value(v.clone())?;
    let a = super::completed_animations::import_parts(&i.animation_input)?;
    completed_skeletons::import(
        &i.rows,
        a.characters
            .into_iter()
            .map(|(id, c)| (id, c.animator))
            .collect(),
        i.provenance["image_sha256"].as_str().unwrap(),
    )
}
fn evaluate(i: Input) -> Result<Value> {
    ensure!(
        i.schema == 1
            && i.contract == CONTRACT
            && i.tick == 88448
            && i.native_parity_ticks == 0
            && i.rows.len() == 3
            && !i.scope.is_empty()
            && i.unqualified.len() >= 5,
        "skeleton component scope/profile differs"
    );
    ensure!(
        i.provenance["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && i.provenance["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c"
            && i.animation_input["provenance"]["image_sha256"] == i.provenance["image_sha256"]
            && i.animation_input["provenance"]["observation_sha256"]
                == i.provenance["observation_sha256"],
        "native skeleton/animator source epoch differs"
    );
    let animation = super::completed_animations::import_parts(&i.animation_input)?;
    let imported = completed_skeletons::import(
        &i.rows,
        animation
            .characters
            .into_iter()
            .map(|(id, c)| (id, c.animator))
            .collect(),
        i.provenance["image_sha256"].as_str().unwrap(),
    )?;
    ensure!(
        imported.keys().copied().collect::<Vec<_>>() == [16, 18, 21],
        "native skeleton people differ"
    );
    let mut bones = 0;
    let mut anchors = 0;
    let mut rows = vec![];
    for (&id, s) in &imported {
        ensure!(
            s.bones.len() == 29 && s.rig.is_none() && !s.demands.is_empty(),
            "skeleton component improperly grants complete rig/pass ownership"
        );
        bones += s.bones.len();
        anchors += s.anchors.values().filter(|a| a.is_some()).count();
        rows.push(json!({"object":id,"bones":s.bones.len(),"native_skin":s.native.skin,"live_instance_qualified":false,"demands":s.demands}));
    }
    ensure!(
        bones == 87 && anchors == 12,
        "native bone/anchor coverage differs"
    );
    let binary = postcard::to_allocvec(&imported)?;
    let cold: BTreeMap<i16, Imported> = postcard::from_bytes(&binary)?;
    ensure!(
        cold == imported && postcard::to_allocvec(&cold)? == binary,
        "retained skeleton parts binary cold roundtrip differs"
    );
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":i.tick,"native_parity_ticks":0,"people":rows,"bones":bones,"native_matrix_anchors":anchors,"unchanged_pose_hierarchy_cache_bits":true,"binary_cold_roundtrip_equal":true,"resumable_baseline":false,"complete_character_qualified":false,"saved_sha256":sims_fixtures::digest(&binary),"unqualified":i.unqualified,"provenance":i.provenance}),
    )
}
pub fn run(store: &Store, m: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "native skeleton fixture contract differs"
    );
    let report = evaluate(serde_json::from_slice(
        &store.read_bytes(&m.source.blob, 16 << 20)?,
    )?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-skeletons.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:87,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("87 native bones/three hierarchies, cache/noise/flag owners and12native Matrix anchors import without pose recomputation or RNG and survive binary cold roundtrip; Live/pass/Rig/Character/World remain unqualified".into()),artifacts:vec![PathBuf::from("completed-skeletons.json")]})
}
