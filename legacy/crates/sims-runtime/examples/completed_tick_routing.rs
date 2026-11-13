//! Compare every actual object-owned routing descriptor word at a source cut.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_routing};
use std::{collections::BTreeMap, fs, path::Path};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5,
        "usage: completed_tick_routing STORE REFS_JSON FIELD NATIVE_ROUTING_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let source: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    ensure!(
        source["tick"] == saved.simulation.state.vm.tick,
        "routing source boundary differs"
    );
    let world = &saved.simulation.state.vm.world;
    let native = completed_routing::import(&source, world)?;
    let actual: BTreeMap<_, _> = world
        .objects
        .iter()
        .map(|(&id, obj)| {
            Ok((
                id,
                obj.routing_slots
                    .clone()
                    .with_context(|| format!("routing owner unknown: {id}"))?,
            ))
        })
        .collect::<Result<_>>()?;
    let a = postcard::to_allocvec(&native)?;
    let b = postcard::to_allocvec(&actual)?;
    let report = json!({"scope":"all retained object-owned routing bank words; not full Session admission",
        "objects":native.len(),"descriptors":native.values().map(Vec::len).sum::<usize>(),
        "matched":a==b,"native_sha256":sims_fixtures::digest(&a),"rust_sha256":sims_fixtures::digest(&b),
        "native_parity_ticks":0,"complete_native_Session_admitted":false});
    let out = Path::new(&args[4]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    ensure!(a == b, "object-owned routing descriptors differ");
    Ok(())
}
