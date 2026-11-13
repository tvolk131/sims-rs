//! Probe an original needs object in the authored household. Failures retain a
//! private trace and state; unsupported services remain failures, never success.
use anyhow::{Result, bail};
use sims_content::Index;
use sims_sim::{Snapshot, StepStop};

fn main() -> Result<()> {
    let selector = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Toilets.iff".into());
    let id = std::env::args()
        .nth(2)
        .map(|s| s.parse::<u16>())
        .transpose()?
        .unwrap_or(16807);
    let motive = std::env::args()
        .nth(3)
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(9);
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = sims_content::household::needs_room(&index, &selector, id, motive)?;
    let output = format!(
        "cache/research/sims-exe/household-object/{}-{id}-{motive}",
        selector.replace(['/', '\\'], "_")
    );
    std::fs::create_dir_all(&output)?;
    let mut trace = vec![];
    let mut recovery = None;
    for _ in 0..20000 {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        let step = s.run_fixture_tick(&p, 1000);
        if let Ok(step) = &step {
            trace.extend(step.trace.clone());
        }
        if step.as_ref().map_or(true, |v| {
            matches!(
                v.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            )
        }) {
            std::fs::write(
                format!("{output}/trace.json"),
                serde_json::to_vec_pretty(&trace)?,
            )?;
            std::fs::write(
                format!("{output}/state.json"),
                serde_json::to_vec_pretty(&s)?,
            )?;
            bail!(
                "needs object stopped at tick {}: {:?}; faults {:?}",
                s.vm.tick,
                step.as_ref().map(|s| &s.stop),
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
        if [
            100, 490, 510, 535, 555, 620, 680, 690, 750, 766, 850, 900, 1000, 1060, 1100, 1200,
            1300, 1500, 1800, 2000, 4000, 5000, 6000, 8000, 10000, 12000, 15000,
        ]
        .contains(&s.vm.tick)
        {
            let bytes = serde_json::to_vec(&Snapshot::capture(&p, &s))?;
            s = Snapshot::from_json(&bytes, &p).map_err(anyhow::Error::msg)?;
            std::fs::write(format!("{output}/tick-{}.json", s.vm.tick), bytes)?;
        }
        if f32::from_bits(s.vm.world.objects[&1].motives.unwrap()[motive]) > 0.
            && recovery.is_none()
        {
            recovery = Some(s.vm.tick);
        }
        if let Some(recovered) = recovery {
            let active = &s.vm.world.actions.as_ref().unwrap().actors[&1].active;
            // Fridge actions hand off to a newly created Food object. A target
            // change alone does not mean that the needs interaction finished.
            if active.is_none() {
                let bytes = serde_json::to_vec(&Snapshot::capture(&p, &s))?;
                Snapshot::from_json(&bytes, &p).map_err(anyhow::Error::msg)?;
                std::fs::write(format!("{output}/completed.json"), bytes)?;
                std::fs::write(
                    format!("{output}/trace.json"),
                    serde_json::to_vec_pretty(&trace)?,
                )?;
                println!(
                    "Need {motive} recovered above zero at tick {recovered}; original action completed at tick {}",
                    s.vm.tick
                );
                return Ok(());
            }
        }
    }
    std::fs::write(
        format!("{output}/trace.json"),
        serde_json::to_vec_pretty(&trace)?,
    )?;
    std::fs::write(
        format!("{output}/state.json"),
        serde_json::to_vec_pretty(&s)?,
    )?;
    bail!("need/action did not recover and complete within the probe limit")
}
