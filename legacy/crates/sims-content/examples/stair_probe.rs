//! Executes original upstairs chair Sit through the original stair portal.
use sims_content::{Index, stairs};
use sims_sim::{Snapshot, StepStop};
use std::io::Write;
fn main() -> anyhow::Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = stairs::laboratory(&index)?;
    for (&id, b) in &s.bodies {
        eprintln!("#{id} {b:?}");
    }
    s.enqueue(&p, 1, 2, 0).map_err(anyhow::Error::msg)?;
    let mut trace = std::fs::File::create("cache/stair-trace.jsonl")?;
    for _ in 0..3000 {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        loop {
            let r = s.run_fixture_tick(&p, 4000);
            if let Ok(r) = &r {
                for t in &r.trace {
                    writeln!(trace, "{}", serde_json::to_string(t)?)?;
                }
                for e in &r.events {
                    eprintln!("tick {} {e:?}", s.vm.tick);
                }
            }
            if r.as_ref().is_err()
                || r.as_ref()
                    .is_ok_and(|r| matches!(r.stop, StepStop::Faulted | StepStop::Animation))
            {
                std::fs::write("cache/stair-failure.json", serde_json::to_vec_pretty(&s)?)?;
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
        if s.vm.tick % 25 == 0 {
            Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
        }
        if s.vm.world.objects[&1].container == Some((2, 0)) {
            std::fs::write(
                "cache/stair-seated.json",
                serde_json::to_vec_pretty(&Snapshot::capture(&p, &s))?,
            )?;
            println!(
                "upstairs chair reached at tick {} on level {}",
                s.vm.tick, s.bodies[&1].level
            );
            return Ok(());
        }
    }
    anyhow::bail!("did not sit upstairs")
}
