//! Retain the persistent person+18c label cache, even when display is disabled.
//! The older0x188-byte resident header stops before this string. Its empty
//! placeholder is not evidence of an empty native cache.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_sim::Simulation;
use sims_vm::ObjectId;
use std::collections::BTreeMap;
pub mod captured;
#[cfg(test)]
mod tests;
pub fn import(source: &Value, reentry: &Value, tick: u64) -> Result<BTreeMap<ObjectId, Vec<u8>>> {
    ensure!(
        source["schema"] == 1
            && source["status"] == "actual_native_person_label_caches_observed"
            && source["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && source["target_executed"] == false
            && [88448, 88449].contains(&tick),
        "native person label source epoch differs"
    );
    let code = &source["source_code"];
    let raw = super::completed_animations::bytes(
        code["raw"].as_str().context("native label code absent")?,
    )?;
    ensure!(
        code["address"] == 0x45ce82
            && code["bytes"] == 0x55
            && raw.len() == 0x55
            && code["sha256"] == "28309fbacaf293333b7a816cebe69c0c8f9c33707db917f569ffec53a3ba76db"
            && sims_fixtures::digest(&raw) == code["sha256"],
        "native label comparison/copy source code differs"
    );
    let controls = reentry["controls"]
        .as_array()
        .context("native person pointer bindings absent")?;
    let cuts = source["cuts"]
        .as_array()
        .context("native person label cuts absent")?;
    ensure!(cuts.len() == 2, "native person label cut cohort differs");
    let mut result = None;
    for (cut, (name, time)) in cuts.iter().zip([("before", 88448), ("after", 88449)]) {
        ensure!(
            cut["cut"] == name && cut["tick"] == time,
            "native label cut identity differs"
        );
        let mut values = BTreeMap::new();
        for row in cut["objects"]
            .as_array()
            .context("native person label objects absent")?
        {
            let id: ObjectId = serde_json::from_value(row["object"].clone())?;
            let owner = controls
                .iter()
                .find(|r| r["object"] == id)
                .context("native label person source absent")?;
            let pointer = owner["evidence"]["native_pointer"]
                .as_u64()
                .context("native person pointer absent")?;
            let raw = super::completed_animations::bytes(
                row["raw"].as_str().context("native label bytes absent")?,
            )?;
            ensure!(
                row["person"] == pointer
                    && row["address"] == pointer + 0x18c
                    && row["bytes"] == raw.len()
                    && (1..=1024).contains(&raw.len())
                    && raw.last() == Some(&0)
                    && !raw[..raw.len() - 1].contains(&0)
                    && row["sha256"] == sims_fixtures::digest(&raw),
                "native label address/string ownership differs"
            );
            ensure!(
                values.insert(id, raw[..raw.len() - 1].to_vec()).is_none(),
                "duplicate native person label"
            );
        }
        ensure!(
            values.keys().copied().eq([16, 18, 21]),
            "native person label cohort differs"
        );
        if time == tick {
            result = Some(values);
        }
    }
    result.context("native label current cut absent")
}
pub fn attach_before(simulation: &mut Simulation, source: &Value, reentry: &Value) -> Result<()> {
    ensure!(
        simulation.vm.tick == 88448,
        "native person labels may only attach at retained baseline"
    );
    let values = import(source, reentry, simulation.vm.tick)?;
    ensure!(
        simulation.characters.keys().eq(values.keys()),
        "resident/label actor cohort differs"
    );
    for (&id, value) in &values {
        let old = &simulation.characters[&id].displayed_animation;
        ensure!(
            old.is_empty() || old == value,
            "native person label has a contradictory retained owner"
        );
    }
    for (id, value) in values {
        simulation
            .characters
            .get_mut(&id)
            .unwrap()
            .displayed_animation = value;
    }
    Ok(())
}
pub fn compare(simulation: &Simulation, source: &Value, reentry: &Value) -> Result<Value> {
    let values = import(source, reentry, simulation.vm.tick)?;
    ensure!(
        simulation.characters.keys().eq(values.keys()),
        "resident/label actor cohort differs"
    );
    let rows: Vec<_> = values
        .iter()
        .map(|(&id, value)| {
            json!({"object":id,"field":"displayed_animation",
        "native":value,"rust":simulation.characters[&id].displayed_animation,
        "matched":*value==simulation.characters[&id].displayed_animation})
        })
        .collect();
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    Ok(
        json!({"schema":1,"tick":simulation.vm.tick,"comparisons":rows,"difference_count":differences,
        "complete_native_Session_admitted":false,"native_parity_ticks":0,
        "scope":"Actual persistent native animation-label strings at person+18c. Disabled rendering does not erase these values."}),
    )
}
