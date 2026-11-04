//! Actual current neighborhood/person-world typed state; no simulation steps.
use super::{AdapterResult, Scope, Status};
use crate::original_session::completed_person_world::{self, Delivery};
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
pub const CONTRACT: &str = "completed-native-person-world-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    native_parity_ticks: u64,
    person_fields: Value,
    native_relationship_orders: BTreeMap<i16, Vec<i16>>,
    family_delivery: Delivery,
    retained_external_family_owners: Value,
    runtime_controls: Value,
    unqualified: Vec<String>,
    provenance: Value,
}
fn evaluate(i: Input) -> Result<Value> {
    ensure!(
        i.schema == 1
            && i.contract == CONTRACT
            && i.tick == 88448
            && i.native_parity_ticks == 0
            && i.unqualified.len() >= 4,
        "person-world component profile differs"
    );
    ensure!(
        i.provenance["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && i.provenance["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "person-world source epoch differs"
    );
    let imported = completed_person_world::import(
        i.person_fields,
        i.native_relationship_orders,
        i.retained_external_family_owners,
        i.family_delivery,
    )?;
    let s = &imported.state;
    ensure!(
        s.neighbors.len() == 463
            && s.families.len() == 26
            && s.object_order.len() == 344
            && s.person_order == [21, 18, 16]
            && s.controlled_family == 5
            && s.tile_chains.len() == 203
            && s.tile_chains.iter().map(Vec::len).sum::<usize>() == 236,
        "original person-world coverage differs"
    );
    let a = s.autonomy.as_ref().unwrap();
    ensure!(
        a.selected_person == Some(21)
            && i.runtime_controls["selected_person"] == 21
            && i.runtime_controls["inventory_query_bit"] == s.inventory_dirty,
        "native selection/UI context differs"
    );
    let bytes = postcard::to_allocvec(s)?;
    let restored: sims_vm::person::WorldState = postcard::from_bytes(&bytes)?;
    ensure!(
        restored == *s && postcard::to_allocvec(&restored)? == bytes,
        "captured person-world binary roundtrip differs"
    );
    Ok(
        json!({"schema":1,"contract":CONTRACT,"status":"passed","scope":"original_component","tick":i.tick,"native_parity_ticks":0,"neighbors":s.neighbors.len(),"families":s.families.len(),"person_order":s.person_order,"selected_person":a.selected_person,"tile_chains":s.tile_chains.len(),"tile_chain_objects":236,"cold_binary_roundtrip_equal":true,"saved_sha256":sims_fixtures::digest(&bytes),"unqualified":i.unqualified,"provenance":i.provenance}),
    )
}
pub fn run(store: &Store, m: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        m.source.format == ArtifactFormat::Json
            && m.source.schema == 1
            && matches!(&m.coverage,Coverage::Component{scope,..}if scope==CONTRACT),
        "person-world fixture contract differs"
    );
    let report = evaluate(serde_json::from_slice(
        &store.read_bytes(&m.source.blob, 16 << 20)?,
    )?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("completed-person-world.json"))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult{status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),observations:463,completed_ticks:0,matched_completed_ticks:0,difference:None,detail:Some("463 native neighbors/26 families and live selection, resource caches, placements and203 tile traversals match typed Rust Person World state; complete World/VM and source consumers remain unqualified".into()),artifacts:vec![PathBuf::from("completed-person-world.json")]})
}
