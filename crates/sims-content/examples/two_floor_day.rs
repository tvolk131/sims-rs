//! Bounded autonomous two-floor acceptance run; retains private diagnostics.
use sims_content::{Index, stairs};
use sims_sim::{Snapshot, StepStop};
fn main() -> anyhow::Result<()> {
    let ticks = std::env::args()
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(43200);
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = stairs::home(&index, 7)?;
    if let Some(path) = std::env::args().nth(2) {
        s = Snapshot::from_json(&std::fs::read(path)?, &p).map_err(anyhow::Error::msg)?;
    }
    let mut crossings = std::collections::BTreeMap::<String, usize>::new();
    for _ in 0..ticks {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        loop {
            let r = s.run_fixture_tick(&p, 4000);
            if let Ok(r) = &r {
                for e in &r.events {
                    if e.detail.starts_with("started animation a2o-stair-") {
                        *crossings
                            .entry(e.detail.split(':').next().unwrap().into())
                            .or_default() += 1;
                    }
                }
            }
            if r.as_ref().is_err()
                || r.as_ref()
                    .is_ok_and(|r| matches!(r.stop, StepStop::Faulted | StepStop::Animation))
            {
                std::fs::write(
                    "cache/two-floor-failure.json",
                    serde_json::to_vec_pretty(&Snapshot::capture(&p, &s))?,
                )?;
                anyhow::bail!(
                    "tick {}: {r:?}; faults {:?}",
                    s.vm.tick,
                    s.vm.threads
                        .values()
                        .filter(|t| matches!(t.status, sims_vm::Status::Faulted(_)))
                        .collect::<Vec<_>>()
                );
            }
            if r.map_err(anyhow::Error::msg)?.stop != StepStop::Budget {
                break;
            }
        }
        s.vm.sounds.clear();
        s.vm.effects.clear();
        s.household.as_mut().unwrap().clock_events.clear();
        if s.vm.tick % 1000 == 0 {
            std::fs::write(
                "cache/two-floor-checkpoint.json",
                serde_json::to_vec(&Snapshot::capture(&p, &s))?,
            )?;
            Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            println!("tick {}: {crossings:?}", s.vm.tick);
        }
    }
    std::fs::write(
        "cache/two-floor-day.json",
        serde_json::to_vec(&Snapshot::capture(&p, &s))?,
    )?;
    println!("Completed {ticks} ticks: {crossings:?}");
    Ok(())
}
