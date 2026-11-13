//! Private-content diagnostic; never substitutes outcomes for original scripts.
use anyhow::{Result, bail};
use sims_content::{Index, household};
use sims_sim::{Body, Point, Snapshot, StepStop};
fn main() -> Result<()> {
    let i = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let args: Vec<_> = std::env::args().collect();
    let (mut p, mut s) = if args.iter().any(|s| s == "--home") {
        household::two_sim_home(&i)?
    } else {
        let (p, mut s) = household::autonomous_household(&i)?;
        household::add_resident(
            &p,
            &mut s,
            13,
            2,
            Body {
                level: 1,
                position: Point::new(104, 72),
                facing: 64,
                radius: 3,
                elevation: 0,
            },
        )?;
        (p, s)
    };
    if args.iter().any(|s| s == "--door") {
        household::add_object(
            &i,
            &mut p,
            &mut s,
            "Doors.iff",
            16815,
            14,
            Body {
                level: 1,
                position: Point::new(104, 136),
                facing: 1,
                radius: 0,
                elevation: 0,
            },
        )?;
    }
    if args.iter().any(|s| s == "--talk") {
        s.vm.world.person.as_mut().unwrap().free_will = false;
        let d = s.vm.world.objects[&13].definition.as_ref().unwrap();
        let entries = &p.modules[&d.source].object_interactions[&d.object];
        let n = entries.iter().position(|e| e.action == 0x2051).unwrap();
        println!("Talk choice {n}: {:?}", s.interaction_choice(&p, 1, 13, n));
        s.enqueue(&p, 1, 13, n).map_err(anyhow::Error::msg)?;
    }
    sims_content::assets::load_linked_animations(&i, &mut p)?;
    let out = "cache/research/sims-exe/two-sim-home";
    std::fs::create_dir_all(out)?;
    if args.iter().any(|s| s == "--dump") {
        for (&id, o) in &s.vm.world.objects {
            if id >= 14 {
                let d = o.definition.as_ref().unwrap();
                let m = &p.modules[&d.source];
                println!(
                    "#{id} {:?} flags {:?} functions {:?} slots {:?}",
                    s.bodies[&id],
                    o.data,
                    m.object_functions.get(&d.object),
                    m.slots.get(&d.slots)
                );
            }
        }
        return Ok(());
    }
    let ticks = args
        .windows(2)
        .find(|w| w[0] == "--ticks")
        .map(|w| w[1].parse())
        .transpose()?
        .unwrap_or(12000);
    let checkpoints = args.iter().any(|s| s == "--checkpoints");
    let mut tail = std::collections::VecDeque::new();
    for tick in 1..=ticks {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        let r = s.run_fixture_tick(&p, 4000);
        if args.iter().any(|s| s == "--events") {
            if let Ok(r) = &r {
                for e in &r.events {
                    println!("{}: {}", e.tick, e.detail);
                }
                for t in &r.trace {
                    if t.location.source.ends_with("HDSI_Talk.iff")
                        || t.location.source.ends_with("SocialDRT.iff")
                        || t.location.source.ends_with("SocialGlobals.iff")
                    {
                        println!("{}: {:?} {}", t.tick, t.location, t.effect);
                    }
                }
            }
        }
        if let Ok(r) = &r {
            tail.extend(r.trace.clone());
            while tail.len() > 500 {
                tail.pop_front();
            }
        }
        if r.as_ref().map_or(true, |r| {
            matches!(
                r.stop,
                StepStop::Faulted | StepStop::Animation | StepStop::Budget
            )
        }) {
            std::fs::write(
                format!("{out}/state.json"),
                serde_json::to_vec_pretty(&Snapshot::capture(&p, &s))?,
            )?;
            if let Ok(r) = &r {
                std::fs::write(format!("{out}/trace.json"), serde_json::to_vec_pretty(r)?)?;
            }
            std::fs::write(
                format!("{out}/tail.json"),
                serde_json::to_vec_pretty(&tail)?,
            )?;
            bail!(
                "tick {tick}: {:?}; {:?}",
                r.as_ref().map(|r| &r.stop),
                s.vm.threads
                    .iter()
                    .filter_map(|(id, t)| if let sims_vm::Status::Faulted(f) = &t.status {
                        Some((id, &f.message))
                    } else {
                        None
                    })
                    .collect::<Vec<_>>()
            );
        }
        s.vm.sounds.clear();
        s.household.as_mut().unwrap().clock_events.clear();
        if checkpoints && tick % 250 == 0 {
            let snap = Snapshot::capture(&p, &s);
            std::fs::write(format!("{out}/checkpoint.json"), serde_json::to_vec(&snap)?)?;
            s = snap
                .restore(&p)
                .map_err(|e| anyhow::anyhow!("tick {tick} restore: {e}"))?;
        }
        if tick % 1000 == 0 {
            println!("tick {tick}: {:?}", s.interactions);
        }
    }
    std::fs::write(
        format!("{out}/completed.json"),
        serde_json::to_vec_pretty(&Snapshot::capture(&p, &s))?,
    )?;
    Ok(())
}
