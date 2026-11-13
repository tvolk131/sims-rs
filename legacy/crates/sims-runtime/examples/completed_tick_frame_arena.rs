//! Diagnose every captured frame-arena word, including inactive storage.
//! Differences must be explained before this can support full-state admission.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_animations};
use std::{collections::BTreeSet, fs, path::Path};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5,
        "usage: completed_tick_frame_arena STORE REFS FIELD NATIVE_THREADS NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let input: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let vm = &saved.simulation.state.vm;
    ensure!(
        input["schema"] == 1 && input["tick"] == vm.tick && input["executes_target_code"] == false,
        "frame-arena source boundary differs"
    );
    let mut differences = Vec::new();
    let mut words = 0;
    let mut cohort = BTreeSet::new();
    for row in input["threads"]["objects"]
        .as_array()
        .context("native threads absent")?
    {
        let id: i16 = row["id"]
            .as_i64()
            .context("native thread id absent")?
            .try_into()?;
        ensure!(cohort.insert(id), "duplicate native thread");
        let t = &vm.threads[vm
            .object_threads
            .get(&id)
            .context("Rust object thread absent")?];
        let a = &row["arena"];
        let raw =
            completed_animations::bytes(a["raw"].as_str().context("native frame arena absent")?)?;
        ensure!(
            raw.len() % 2 == 0
                && a["bytes"] == raw.len()
                && a["sha256"] == sims_fixtures::digest(&raw),
            "native frame arena receipt differs"
        );
        let native: Vec<_> = raw
            .chunks_exact(2)
            .map(|b| Some(i16::from_le_bytes(b.try_into().unwrap())))
            .collect();
        words += native.len();
        if native.len() != t.frame_memory.len() {
            differences.push(json!({"object":id,"field":"arena_length","native":native.len(),"rust":t.frame_memory.len()}));
        }
        for (i, (a, b)) in native.iter().zip(&t.frame_memory).enumerate() {
            if a != b {
                let active = t.stack.iter().position(|f| {
                    i >= f.storage_offset
                        && i < f.storage_offset + 8 + f.args.len() + f.locals.len()
                });
                differences
                    .push(json!({"object":id,"word":i,"native":a,"rust":b,"active_frame":active}));
            }
        }
    }
    ensure!(
        cohort.iter().eq(vm.world.objects.keys()),
        "frame arena cohort differs"
    );
    let report = json!({"schema":1,"tick":vm.tick,"objects":cohort.len(),"words":words,
        "difference_count":differences.len(),"differences":differences,"native_parity_ticks":0,
        "complete_native_Session_admitted":false,
        "scope":"Raw frame_memory versus all original arena words; active Frame overlays are not yet normalized or excluded"});
    let out = Path::new(&args[4]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"words":words,"differences":report["difference_count"]})
    );
    ensure!(
        report["difference_count"] == 0,
        "raw native frame arenas differ"
    );
    Ok(())
}
