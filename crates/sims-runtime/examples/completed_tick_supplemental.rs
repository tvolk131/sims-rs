//! Source-derived score, contained-slot and BodyStrings owner comparisons.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{
    Saved,
    completed_supplemental::{self, Geometry},
};
use sims_vm::{ObjectId, slot_geometry::Anchor};
use std::{collections::BTreeMap, fs, path::Path};

fn same<T: Serialize>(name: &str, native: &T, rust: &T, rows: &mut Vec<Value>) -> Result<()> {
    let a = postcard::to_allocvec(native)?;
    let b = postcard::to_allocvec(rust)?;
    let mut row = json!({"field":name,"matched":a==b,"native_sha256":sims_fixtures::digest(&a),
        "rust_sha256":sims_fixtures::digest(&b),"first_byte_offset":a.iter().zip(&b).position(|(a,b)|a!=b)});
    if a != b {
        row["native"] = serde_json::to_value(native)?;
        row["rust"] = serde_json::to_value(rust)?;
    }
    rows.push(row);
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5,
        "usage: completed_tick_supplemental STORE REFS FIELD NATIVE_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let source: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    ensure!(
        source["tick"] == saved.simulation.state.vm.tick,
        "supplemental comparison boundary differs"
    );
    let world = &saved.simulation.state.vm.world;
    let before = postcard::to_allocvec(world)?;
    let expected = completed_supplemental::import(&source, world)?;
    ensure!(
        before == postcard::to_allocvec(world)?,
        "supplemental import mutated VM/RNG world"
    );
    let scores: BTreeMap<_, _> = world
        .objects
        .iter()
        .filter(|(_, o)| o.person.is_some())
        .map(|(&id, o)| {
            Ok((
                id,
                o.native_motive_scores
                    .as_ref()
                    .context("retained personal score owner unknown")?
                    .clone(),
            ))
        })
        .collect::<Result<_>>()?;
    let mut geometry = BTreeMap::<ObjectId, Vec<Geometry>>::new();
    let mut heads = BTreeMap::new();
    for (&id, o) in &world.objects {
        let slots = o
            .native_slot_geometry
            .as_ref()
            .context("retained contained-slot geometry unknown")?;
        geometry.insert(
            id,
            slots
                .iter()
                .map(|s| Geometry {
                    offset: s.offset,
                    height: s.height,
                })
                .collect(),
        );
        if o.person.is_some() {
            let anchor = slots
                .get(1)
                .and_then(|s| s.anchor.as_ref())
                .context("person HEAD anchor unknown")?;
            let Anchor::Named(name) = anchor else {
                anyhow::bail!("person HEAD anchor absent")
            };
            heads.insert(id, name.clone());
        }
    }
    let mut rows = Vec::new();
    same(
        "personal_score_caches",
        &expected.scores,
        &scores,
        &mut rows,
    )?;
    same(
        "all_contained_slot_offsets_and_heights",
        &expected.geometry,
        &geometry,
        &mut rows,
    )?;
    same(
        "person_dependent_HEAD_names",
        &expected.head_names,
        &heads,
        &mut rows,
    )?;
    same(
        "current_BodyStrings_caches_and_publications",
        &expected.body_profiles,
        &world.body_profiles,
        &mut rows,
    )?;
    let differences = rows.iter().filter(|r| r["matched"] != true).count();
    let report = json!({"schema":1,"tick":saved.simulation.state.vm.tick,"objects":geometry.len(),
        "slots":geometry.values().map(Vec::len).sum::<usize>(),"people":scores.len(),
        "difference_count":differences,"comparisons":rows,"native_parity_ticks":0,
        "complete_native_Session_admitted":false,"vm_world_unchanged":true,
        "opaque_uninitialized_non_HEAD_slot_words":expected.opaque_slot_words,
        "exclusion":"Native initializer46de93 and converter46debb leave record+20 untouched; person setup45962b defines HEAD only for logical slot1, consumed by58f8ff. Other slot+20 pointer words remain raw provenance and are not treated as names for this recorded tick. This is not qualification of every future slot consumer.",
        "scope":"Native source-derived supplemental owners, not complete Session admission"});
    let out = Path::new(&args[4]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"comparisons":rows.len(),"differences":differences})
    );
    ensure!(differences == 0, "retained supplemental owners differ");
    Ok(())
}
