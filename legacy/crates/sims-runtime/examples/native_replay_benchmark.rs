//! Controlled headless replay timing; no original execution or GUI is launched.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_fixtures::Store;
use sims_runtime::conformance::native_household::{Checks, Prepared};
use std::{fs, path::Path, time::Instant};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        (4..=6).contains(&a.len()),
        "usage: native_replay_benchmark STORE CASE IMMUTABLE NEW_DIR [REPETITIONS] [cold]"
    );
    let n: usize = a.get(4).map(|s| s.parse()).transpose()?.unwrap_or(3);
    ensure!((1..=20).contains(&n), "repetitions must be1..20");
    let checks = match a.get(5).map(String::as_str) {
        None => Checks::Replay,
        Some("cold") => Checks::Cold,
        _ => anyhow::bail!("unsupported benchmark mode"),
    };
    let start = Instant::now();
    let store = Store::open(&a[0])?;
    let case = store.load_case(&a[1])?;
    let integrity = start.elapsed().as_secs_f64() * 1000.;
    let prepared = Prepared::load(
        &store,
        &case,
        Path::new(&a[2]),
        "UserData/Houses/House05.iff",
    )?
    .context("qualified native fixture unavailable")?;
    fs::create_dir(&a[3])?;
    let mut rows = vec![];
    for i in 0..n {
        let start = Instant::now();
        let report = prepared.replay(checks)?;
        let ms = start.elapsed().as_secs_f64() * 1000.;
        fs::write(
            Path::new(&a[3]).join(format!("replay-{i}.json")),
            serde_json::to_vec_pretty(&report)?,
        )?;
        ensure!(
            report.difference.is_none() && report.matched_completed_ticks == 1,
            "native benchmark diverged"
        );
        rows.push(
            json!({"iteration":i,"restore_simulate_compare_ms":ms,"phases_ms":report.timings_ms}),
        );
    }
    let result = json!({"schema":1,"case":case.id,"build_profile":if cfg!(debug_assertions){"debug"}else{"release"},"checks":checks,"fixture_integrity_ms":integrity,"prepare_ms":prepared.preparation_ms,"runs":rows,"matched_reference_ticks":1,"source_qualification_reused":true,"scope":"Same source-qualified reference and complete mapped state/RNG/trace checks per repetition; setup and simulation separated. Throughput is for this one-tick fixture, not whole-game or GUI performance."});
    fs::write(
        Path::new(&a[3]).join("benchmark.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
