//! Actual immutable live-skeleton identity and original initializer ownership.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::{
    conformance,
    original_session::{Saved, completed_animations},
};
use std::{fs, path::Path};

fn same<T: Serialize>(
    rows: &mut Vec<Value>,
    id: i16,
    field: &str,
    native: &T,
    rust: &T,
) -> Result<()> {
    let a = postcard::to_allocvec(native)?;
    let b = postcard::to_allocvec(rust)?;
    rows.push(json!({"object":id,"field":field,"matched":a==b,"native_sha256":sims_fixtures::digest(&a),"rust_sha256":sims_fixtures::digest(&b)}));
    Ok(())
}
fn evaluate(saved: &Saved, prototypes: &[u8], proof: &Value, current: &Value) -> Result<Value> {
    ensure!(
        [88448, 88449].contains(&saved.simulation.state.vm.tick)
            && current["tick"] == saved.simulation.state.vm.tick,
        "native live-source cut differs"
    );
    ensure!(
        proof["schema"] == 1
            && proof["status"] == "native_shared_asset_caches_unchanged_qualified"
            && proof["before_tick"] == 88448
            && proof["after_tick"] == 88449
            && proof["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && proof["coverage_payload_sha256"]
                == "e308ce89276b8d775a1820c160e495f7fde01491eea1a5b7621513ce8d18e393"
            && proof["prototype_source_sha256"] == sims_fixtures::digest(prototypes)
            && proof["target_executed"] == false,
        "native live-source cache qualification differs"
    );
    let fields = proof["fields"]
        .as_array()
        .context("native shared fields absent")?;
    ensure!(
        fields.len() == 1573 && fields.iter().all(|r| r["before_after_equal"] == true),
        "native shared cache fields changed"
    );
    let input: Value = serde_json::from_slice(prototypes)?;
    let source = conformance::completed_prototypes::import_parts(&input)?;
    let actors = input["graph"]["actors"]
        .as_array()
        .context("native prototype actors absent")?;
    let geometry = current["geometry"]["characters"]
        .as_array()
        .context("actual current model owners absent")?;
    ensure!(
        source
            .characters
            .residents
            .keys()
            .eq(saved.simulation.state.characters.keys())
            && source
                .characters
                .residents
                .keys()
                .eq(saved.assets.characters.keys()),
        "live source cohort differs"
    );
    let mut rows = vec![];
    for (&id, native) in &source.characters.residents {
        let a = actors
            .iter()
            .find(|a| a["object"] == id)
            .context("native prototype lookup actor absent")?;
        let g = geometry
            .iter()
            .find(|g| g["object"] == id)
            .context("current model owner absent")?;
        let raw = completed_animations::bytes(
            g["model_raw"]
                .as_str()
                .context("current model bytes absent")?,
        )?;
        ensure!(
            g["model"] == a["model"] && g["skin"] == a["live_skin"] && raw.len() == 0xb0,
            "current prototype/live model alias differs"
        );
        let manager = u32::from_le_bytes(raw[0..4].try_into()?);
        ensure!(a["manager"] == manager, "current model manager differs");
        // The original before/after source proof includes this exact alias word
        // separately from the six changing live model/skin headers.
        let address = g["model"].as_u64().context("native model pointer absent")?;
        ensure!(
            fields.iter().any(|r| r["address"] == address
                && r["bytes"] == 4
                && r["before_after_equal"] == true),
            "current model manager alias lacks qualified source ownership"
        );
        let skeleton = native
            .source
            .as_ref()
            .context("native live source missing")?;
        same(
            &mut rows,
            id,
            "complete_live_immutable_skeleton_source",
            &native.source,
            &saved.simulation.state.characters[&id].source,
        )?;
        same(
            &mut rows,
            id,
            "asset_skeleton_name",
            &skeleton.name,
            &saved.assets.characters[&id].skeleton,
        )?;
        same(
            &mut rows,
            id,
            "complete_original_layer_initializer_settings",
            &native.controls.settings,
            &saved.simulation.state.characters[&id].controls.settings,
        )?;
    }
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    Ok(
        json!({"schema":1,"tick":saved.simulation.state.vm.tick,"comparison_count":rows.len(),"comparisons":rows,"difference_count":differences,"complete_native_Session_admitted":false,"native_parity_ticks":0,
        "scope":"All live immutable skeleton metadata, source names and original layer settings, tied to captured manager/node lookup and qualified unchanged registry/lookup/catalog bytes plus actual current model/skin aliases. Shared mesh aliases, lifetime and final Session admission remain separate."}),
    )
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 7,
        "usage: completed_tick_character_sources STORE REFS FIELD PROTOTYPES CACHE_PROOF CURRENT_NATIVE_PARTS NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let report = evaluate(
        &saved,
        &fs::read(&a[3])?,
        &serde_json::from_slice(&fs::read(&a[4])?)?,
        &serde_json::from_slice(&fs::read(&a[5])?)?,
    )?;
    fs::create_dir(Path::new(&a[6]))?;
    fs::write(
        Path::new(&a[6]).join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "comparisons {}, differences {}",
        report["comparison_count"], report["difference_count"]
    );
    ensure!(
        report["difference_count"] == 0,
        "native live source ownership differs"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires private captured Goth source catalog, cache proof and managed baseline"]
    fn actual_live_sources_reject_missing_source_changed_name_settings_and_manager() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(run)?
            .join()
            .map_err(|_| anyhow::anyhow!("live source test worker panicked"))?
    }
    fn run() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |p: &str| -> Result<Vec<u8>> { Ok(fs::read(root.join(p))?) };
        let cut = root.join("fixtures/private/goth-label-owners-20261029-first/session-first");
        let refs: Value = serde_json::from_slice(&fs::read(cut.join("refs.json"))?)?;
        let saved: Saved = Store::open(cut.join("store"))?
            .get_value(&serde_json::from_value(refs["session"].clone())?)?;
        let prototypes =
            read("fixtures/private/goth-resource-closure-20261028-first/prototypes-input.json")?;
        let proof: Value = serde_json::from_slice(&read(
            "fixtures/private/goth-source-cache-contract-20261029-first/cache-proof-second.json",
        )?)?;
        let current: Value = serde_json::from_slice(&read(
            "fixtures/private/goth-native-animation-rebase-20261028-first/native-before-complete-geometry-first.json",
        )?)?;
        ensure!(
            evaluate(&saved, &prototypes, &proof, &current)?["difference_count"] == 0,
            "actual live sources differ"
        );
        let vm = postcard::to_allocvec(&saved.simulation.state.vm)?;
        for kind in 0..3 {
            let mut changed = saved.clone();
            match kind {
                0 => {
                    changed
                        .simulation
                        .state
                        .characters
                        .get_mut(&16)
                        .unwrap()
                        .source = None
                }
                1 => changed
                    .assets
                    .characters
                    .get_mut(&16)
                    .unwrap()
                    .skeleton
                    .push('x'),
                2 => {
                    changed
                        .simulation
                        .state
                        .characters
                        .get_mut(&16)
                        .unwrap()
                        .controls
                        .settings
                        .fade_duration += 1
                }
                _ => unreachable!(),
            }
            ensure!(
                evaluate(&changed, &prototypes, &proof, &current)?["difference_count"] == 1,
                "live source mutation missed: {kind}"
            );
            ensure!(
                postcard::to_allocvec(&changed.simulation.state.vm)? == vm,
                "source comparison changed VM/RNG"
            );
        }
        let mut wrong = current.clone();
        let r = &mut wrong["geometry"]["characters"][0]["model_raw"];
        let mut b = completed_animations::bytes(r.as_str().unwrap())?;
        b[..4].fill(0);
        *r = b
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            .into();
        ensure!(
            evaluate(&saved, &prototypes, &proof, &wrong).is_err(),
            "wrong native manager was accepted"
        );
        Ok(())
    }
}
