//! Stored retained cuts against each pinned original boundary. No VM executes.
use super::*;
use crate::original_session::completed_assets::checkpoints::Retained;
use sims_fixtures::{BlobRef, Store, readset::Readset};
use std::path::Path;

pub fn components(
    state: &Value,
    memory: &Readset<'_>,
    cut: &Retained,
    hash: &str,
) -> Result<Value> {
    let scalar = super::compare(state, &cut.simulation.state)?;
    let animation = super::animation::compare(state, memory, &cut.simulation.state, hash)?;
    let pose = super::pose::compare(
        &state,
        &memory,
        &cut.simulation.state,
        &cut.characters,
        &cut.meshes,
        &cut.native_meshes,
        hash,
    )?;
    let physical = super::physical::compare(state, memory, &cut.simulation.state)?;
    let routes = super::routes::compare(state, memory, &cut.simulation.state)?;
    let sound = cut
        .presentation
        .disabled_sound
        .as_ref()
        .context("retained sound owner absent")?;
    sound.validate_current(state, memory)?;
    let matched = scalar["differences"] == 0
        && animation["differences"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && pose["differences"] == 0
        && physical["differences"] == 0
        && routes["differences"] == 0;
    Ok(
        json!({"matched":matched,"scalar":scalar,"animation":animation,"pose":pose,"physical":physical,"routes":routes,"disabled_sound":"current original owner validated"}),
    )
}

pub fn compare(report: &Value, store: &Store, native: &Path) -> Result<Value> {
    compare_policy(report,store,native,true)
}
pub fn analyze_all(report: &Value, store: &Store, native: &Path) -> Result<Value> {
    compare_policy(report,store,native,false)
}
fn compare_policy(report: &Value, store: &Store, native: &Path,stop_at_first:bool) -> Result<Value> {
    let seal_raw = std::fs::read(native.join("capture-result.json"))?;
    ensure!(
        sims_fixtures::digest(&seal_raw)
            == "4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393",
        "component matrix native seal differs"
    );
    let seal: Value = serde_json::from_slice(&seal_raw)?;
    let mut cuts = vec![serde_json::from_value::<BlobRef>(
        report["baseline_retained_cut"].clone(),
    )?];
    for row in report["boundary_sequence"]
        .as_array()
        .context("retained boundary sequence absent")?
    {
        ensure!(
            row.get("status").is_none() && row["boundary"] == cuts.len(),
            "unfinished or misordered retained boundary"
        );
        cuts.push(serde_json::from_value(row["retained_cut"].clone())?);
    }
    let mut rows = Vec::new();
    for (index, reference) in cuts.iter().enumerate() {
        let cut: Retained = store.get_value(reference)?;
        let path = native.join(format!("completed-{index:02}"));
        let encoded = std::fs::read(path.join("observation.json.gz"))?;
        let hash = sims_fixtures::digest(&encoded);
        ensure!(
            seal["observations"][index]["sha256"] == hash,
            "component matrix original observation differs"
        );
        let state: Value = serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw = std::fs::read(path.join("acknowledged.mem"))?;
        let memory = Readset::parse(&raw)?;
        ensure!(
            state["tick"] == 89073 + index && state["tick"] == cut.simulation.state.vm.tick,
            "matrix tick alignment differs"
        );
        let component = components(&state, &memory, &cut, &hash)?;
        let matched = component["matched"] == true;
        let mut row = component;
        row["index"] = json!(index);
        row["tick"] = state["tick"].clone();
        row["source_sha256"] = json!(hash);
        row["retained_cut"] = json!(reference);
        rows.push(row);
        eprintln!("stored boundary {index} component comparison: {matched}");
        if !matched&&stop_at_first {
            break;
        }
    }
    Ok(
        json!({"schema":1,"kind":"stored_retained_component_matrix","native_parity_credit":0,
        "requested_boundaries":cuts.len(),"checked_boundaries":rows.len(),"matched":rows.iter().all(|r|r["matched"]==true),"rows":rows,
        "qualification":"Selected observed scalar/stack, animation, pose, CPU geometry, physical and exposed route-owner fields. Ordinary graphics, full route-candidate ownership, complete source/Session and normal-runner admission remain separate."}),
    )
}
