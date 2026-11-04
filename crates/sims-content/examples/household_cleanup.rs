use anyhow::{Result, bail};
use sims_content::{Index, household};
use sims_sim::{Snapshot, StepStop};
fn main() -> Result<()> {
    let i = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = household::autonomous_household(&i)?;
    let mut target = None;
    let mut emptying = false;
    let mut trace = std::collections::VecDeque::new();
    let out = "cache/research/sims-exe/household-cleanup";
    std::fs::create_dir_all(out)?;
    for _ in 0..20000 {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        let step = s.run_fixture_tick(&p, 2000);
        if let Ok(r) = &step {
            trace.extend(r.trace.clone());
            while trace.len() > 100 {
                trace.pop_front();
            }
        }
        if step.as_ref().map_or(true, |r| {
            matches!(
                r.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            )
        }) {
            std::fs::write(format!("{out}/state.json"), serde_json::to_vec_pretty(&s)?)?;
            std::fs::write(
                format!("{out}/trace.json"),
                serde_json::to_vec_pretty(&trace)?,
            )?;
            bail!(
                "cleanup at {}: {:?}; {:?}",
                s.vm.tick,
                step,
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
        if target.is_none()
            && s.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .is_none()
        {
            if let Some((&id, _)) =
                s.vm.world
                    .objects
                    .iter()
                    .find(|(_, o)| o.definition.as_ref().is_some_and(|d| d.guid == 0x7f907075))
            {
                s.vm.world.person.as_mut().unwrap().free_will = false;
                s.enqueue(&p, 1, id, 0).map_err(anyhow::Error::msg)?;
                target = Some(id);
                println!("Queued cleanup for #{id} at {}", s.vm.tick);
            }
        }
        s.vm.sounds.clear();
        s.household.as_mut().unwrap().clock_events.clear();
        if s.vm.tick % 100 == 0 {
            s = Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(id) = target {
            if !s.vm.world.objects.contains_key(&id)
                && s.vm.world.actions.as_ref().unwrap().actors[&1]
                    .active
                    .is_none()
            {
                if !emptying {
                    let d = s.vm.world.objects[&11].definition.as_ref().unwrap().clone();
                    // Use the original diagnostic Force Full tree to prepare this test's full bin.
                    let fill =
                        s.vm.spawn(
                            &p,
                            sims_vm::Start {
                                owner: &d.source,
                                bhav: 4114,
                                caller: 11,
                                callee: 11,
                                stack_object: 11,
                                args: vec![0; 4],
                                check: false,
                            },
                        )
                        .map_err(anyhow::Error::msg)?;
                    let r = s.vm.run_slice(&p, fill, 100).map_err(anyhow::Error::msg)?;
                    if !matches!(r.stop, sims_vm::Stop::Returned(true)) {
                        bail!("Force Full failed: {r:?}");
                    }
                    s.vm.threads.remove(&fill);
                    let d = s.vm.world.objects[&11].definition.as_ref().unwrap();
                    let menu = p.modules[&d.source].object_interactions[&d.object]
                        .iter()
                        .position(|e| e.action == 4100)
                        .unwrap();
                    s.enqueue(&p, 1, 11, menu).map_err(anyhow::Error::msg)?;
                    emptying = true;
                    println!("Trash disposed; queued Empty bin at {}", s.vm.tick);
                    continue;
                }
                if !s.pending_interactions(1).unwrap().is_empty()
                    || s.vm.world.objects[&11].attributes[0] != 0
                {
                    continue;
                }
                std::fs::write(
                    format!("{out}/completed.json"),
                    serde_json::to_vec(&Snapshot::capture(&p, &s))?,
                )?;
                println!("Cleanup completed at {}", s.vm.tick);
                return Ok(());
            }
        }
    }
    std::fs::write(format!("{out}/state.json"), serde_json::to_vec_pretty(&s)?)?;
    std::fs::write(
        format!("{out}/trace.json"),
        serde_json::to_vec_pretty(&trace)?,
    )?;
    bail!("cleanup did not complete")
}
