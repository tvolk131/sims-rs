//! Native persistent Simulation inputs, qualified by complete readset rereads.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_animations, completed_simulation};
use std::{collections::BTreeMap, fs, path::Path};
fn bytes(v: &Value) -> Result<Vec<u8>> {
    completed_animations::bytes(v.as_str().context("owner bytes absent")?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn half(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn same<T: Serialize>(name: &str, a: &T, b: &T, rows: &mut Vec<Value>) -> Result<()> {
    let x = postcard::to_allocvec(a)?;
    let y = postcard::to_allocvec(b)?;
    let mut r = json!({"field":name,"matched":x==y,"native_sha256":sims_fixtures::digest(&x),"rust_sha256":sims_fixtures::digest(&y)});
    if x != y {
        r["native"] = serde_json::to_value(a)?;
        r["rust"] = serde_json::to_value(b)?;
    }
    rows.push(r);
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 6,
        "usage: completed_tick_simulation_owners STORE REFS FIELD BASELINE_REFS NATIVE_JSON NEW_DIR"
    );
    let store = Store::open(&args[0])?;
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved = store.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let baseline_refs: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let baseline: Saved =
        store.get_value(&serde_json::from_value(baseline_refs["session"].clone())?)?;
    let input: Value = serde_json::from_slice(&fs::read(&args[4])?)?;
    let s = &saved.simulation.state;
    ensure!(
        input["schema"] == 1
            && input["tick"] == s.vm.tick
            && input["executes_target_code"] == false
            && input["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "Simulation owner source boundary differs"
    );
    let source =
        completed_simulation::import_vm(&input["original_before"], &baseline.simulation.state.vm)?;
    let old = input["original_before"]["reads"]
        .as_array()
        .context("qualified before readset absent")?;
    let current = input["reads"]
        .as_array()
        .context("actual cut readset absent")?;
    ensure!(
        old.len() == current.len(),
        "Simulation owner reread extent differs"
    );
    let mut simulator = None;
    let mut changed = Vec::new();
    for (a, b) in old.iter().zip(current) {
        let raw = bytes(&b["raw"])?;
        ensure!(
            a["label"] == b["label"]
                && a["address"] == b["address"]
                && a["bytes"] == b["bytes"]
                && b["bytes"] == raw.len()
                && b["sha256"] == sims_fixtures::digest(&raw),
            "Simulation owner reread alias/hash differs"
        );
        if a["raw"] != b["raw"] {
            ensure!(
                a["label"] == "simulation.simulator",
                "changed cached input requires independent after decoder"
            );
            changed.push(b["label"].clone());
        }
        if a["label"] == "simulation.simulator" {
            ensure!(raw.len() == 0x1b0, "simulator extent differs");
            simulator = Some(raw);
        }
    }
    let sim = simulator.context("actual simulator owner absent")?;
    let clock = sims_sim::clock::LotClock {
        hour: half(&sim, 0x10),
        day: half(&sim, 0x12),
        period: half(&sim, 0x18),
        minute: half(&sim, 0x1a),
        second: half(&sim, 0x1c),
        month: half(&sim, 0x1e),
        year: half(&sim, 0x20),
        elapsed_days: half(&sim, 0x48),
        month_days: half(&sim, 0x4a),
        ticks: word(&sim, 0x98),
        statistics: std::array::from_fn(|i| {
            std::array::from_fn(|j| word(&sim, 0xe4 + i * 32 + j * 4) as i32)
        }),
    };
    clock.validate().map_err(anyhow::Error::msg)?;
    let mut profiles = BTreeMap::new();
    for row in input["original_before"]["people"]
        .as_array()
        .context("native score owners absent")?
    {
        let id: i16 = row["object"]
            .as_i64()
            .context("score id absent")?
            .try_into()?;
        let age = half(&bytes(&row["age_raw"])?, 0);
        let adult = age == 0 || age > 17;
        let curves = source
            .person_scores
            .get(&id)
            .context("score actor absent")?;
        if let Some(previous) = profiles.insert(adult, curves.clone()) {
            ensure!(
                previous == *curves,
                "household compatibility score profiles differ"
            );
        }
    }
    let tuning = sims_vm::motives::Tuning {
        update_precision: sims_vm::motives::Precision::Single,
        rates: source.rates,
        adult_mood: source.adult_mood,
        child_mood: source.child_mood,
        adult_scores: profiles
            .remove(&true)
            .context("adult score profile absent")?,
        child_scores: profiles
            .remove(&false)
            .context("child score profile absent")?,
    };
    let h = s.household.as_ref().context("retained household absent")?;
    let mut rows = Vec::new();
    same(
        "lot_calendar_and_all_daily_statistics",
        &clock,
        &h.clock,
        &mut rows,
    )?;
    same(
        "cached_tuning_rates_mood_and_score_profiles",
        &tuning,
        &h.tuning,
        &mut rows,
    )?;
    same(
        "application_house_statistics",
        &Some(source.statistics),
        &h.house_statistics,
        &mut rows,
    )?;
    let valuation = sims_sim::house_valuation::Retained {
        objects_value: word(&sim, 0xa0) as i32,
        architecture_value: word(&sim, 0xa4) as i32,
    };
    same(
        "retained_lot_valuation",
        &Some(valuation),
        &h.valuation,
        &mut rows,
    )?;
    same(
        "native_character_settings_all_fields",
        &Some(source.character),
        &s.native_character_settings,
        &mut rows,
    )?;
    same(
        "native_placement_diagnostics",
        &source.diagnostics,
        &s.native_lot
            .as_ref()
            .context("native lot absent")?
            .diagnostics,
        &mut rows,
    )?;
    same(
        "native_scheduler_cursor",
        &(word(&sim, 0x1a8) as i32),
        &s.native_scheduler
            .as_ref()
            .context("native scheduler absent")?
            .cursor,
        &mut rows,
    )?;
    let differences = rows.iter().filter(|r| r["matched"] != true).count();
    let report = json!({"schema":1,"tick":s.vm.tick,"comparisons":rows,"difference_count":differences,"changed_readsets":changed,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All qualified persistent Simulation settings and actual calendar; source before inputs are used only after every non-simulator byte is independently proved unchanged at this actual cut"});
    let out = Path::new(&args[5]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"comparisons":rows.len(),"differences":differences})
    );
    ensure!(
        differences == 0,
        "retained persistent Simulation owners differ"
    );
    Ok(())
}
