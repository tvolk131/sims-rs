//! Full-day diagnostic runner. A successful run is evidence for this authored
//! scenario, not a claim of complete native lot or expansion compatibility.
use anyhow::{Context, Result, bail};
use sims_content::{Index, household};
use sims_sim::{Snapshot, StepStop};
use std::{collections::VecDeque, io::Write};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = household::autonomous_household(&index)?;
    let output = "cache/research/sims-exe/household-day";
    std::fs::create_dir_all(output)?;
    let mut trace = VecDeque::new();
    let mut timeline = vec![];
    let mut previous = None;
    let mut log = std::fs::File::create(format!("{output}/progress.jsonl"))?;
    let ticks = std::env::args()
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(129600);
    for _ in 0..ticks {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        let step = s.run_fixture_tick(&p, 2000);
        if let Ok(step) = &step {
            trace.extend(step.trace.clone());
            while trace.len() > 200 {
                trace.pop_front();
            }
        }
        if step.as_ref().map_or(true, |v| {
            matches!(
                v.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            )
        }) {
            std::fs::write(
                format!("{output}/state.json"),
                serde_json::to_vec_pretty(&s)?,
            )?;
            std::fs::write(
                format!("{output}/trace-tail.json"),
                serde_json::to_vec_pretty(&trace)?,
            )?;
            bail!(
                "household stopped at tick {}: {:?}; faults {:?}",
                s.vm.tick,
                step.as_ref().map(|v| &v.stop),
                s.vm.threads
                    .values()
                    .filter_map(|t| if let sims_vm::Status::Faulted(f) = &t.status {
                        Some(f)
                    } else {
                        None
                    })
                    .collect::<Vec<_>>()
            );
        }
        let action = s.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .clone();
        let changed = previous
            .as_ref()
            .map(|a: &sims_vm::action::Action| (a.id, a.target, a.menu_id))
            != action.as_ref().map(|a| (a.id, a.target, a.menu_id));
        if changed || s.vm.tick % 1000 == 0 {
            let row = serde_json::json!({"tick":s.vm.tick,"action":action,"clock":s.household.as_ref().unwrap().clock,"motives":s.vm.world.objects[&1].motives.unwrap().map(f32::from_bits),"objects":s.vm.world.objects.len()});
            writeln!(log, "{}", row)?;
            log.flush()?;
            timeline.push(row);
        }
        if changed
            || s.vm.tick % 2000 == 0
            || [778, 800, 900, 1000, 1100, 4200, 4400, 18000, 26000, 40000].contains(&s.vm.tick)
        {
            let snapshot = Snapshot::capture(&p, &s);
            std::fs::write(
                format!("{output}/tick-{}.json", s.vm.tick),
                serde_json::to_vec(&snapshot)?,
            )?;
            s = snapshot
                .restore(&p)
                .map_err(|error| {
                    let _ = std::fs::write(
                        format!("{output}/trace-tail.json"),
                        serde_json::to_vec_pretty(&trace).unwrap(),
                    );
                    anyhow::Error::msg(error)
                })
                .with_context(|| format!("checkpoint validation at tick {}", s.vm.tick))?;
        }
        previous = action;
        // Presentation queues are consumed by this headless client.
        s.vm.sounds.clear();
        s.household.as_mut().unwrap().clock_events.clear();
    }
    std::fs::write(
        format!("{output}/timeline.json"),
        serde_json::to_vec_pretty(&timeline)?,
    )?;
    std::fs::write(
        format!("{output}/completed.json"),
        serde_json::to_vec(&Snapshot::capture(&p, &s))?,
    )?;
    println!(
        "Completed {} ticks; {} timeline checkpoints",
        s.vm.tick,
        timeline.len()
    );
    Ok(())
}
