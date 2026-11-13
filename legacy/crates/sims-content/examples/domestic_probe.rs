//! Private-content diagnostic runner; writes failures and trace to ignored cache.
use sims_content::{Index, domestic};
use sims_sim::{Snapshot, StepStop};
use std::{collections::BTreeSet, io::Write};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = domestic::home(&index)?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    let activity = args.first().map(String::as_str).unwrap_or("meal");
    let ticks: u64 = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(15000);
    for (&id, o) in &s.vm.world.objects {
        let d = o.definition.as_ref().unwrap();
        eprintln!(
            "#{id} {} OBJD {} {:?} menus {:?}",
            d.source,
            d.object,
            s.bodies.get(&id),
            p.modules[&d.source]
                .object_interactions
                .get(&d.object)
                .map(|m| m.iter().map(|m| (&m.label, m.action)).collect::<Vec<_>>())
        );
    }
    if activity == "slots" {
        for id in [26, 27, 28, 29, 30, 31] {
            let d = s.vm.world.objects[&id].definition.as_ref().unwrap();
            println!("#{id} {:?}", p.modules[&d.source].slots.get(&d.slots));
        }
        for m in p
            .modules
            .values()
            .filter(|m| m.source.ends_with("Food.iff"))
        {
            println!("food menus {:?}", m.object_interactions);
        }
        return Ok(());
    }
    s.vm.world.person.as_mut().unwrap().free_will = activity == "auto";
    if matches!(activity, "sleep" | "sleep-pair") {
        for id in if activity == "sleep-pair" {
            vec![1, 13]
        } else {
            vec![1]
        } {
            s.vm.world
                .objects
                .get_mut(&id)
                .unwrap()
                .motives
                .as_mut()
                .unwrap()[5] = (-30f32).to_bits();
        }
    }
    if activity != "auto" {
        let (target, bhav) = if matches!(activity, "sleep" | "sleep-pair") {
            (20, 0x100a)
        } else if activity == "serve" {
            (4, 0x100b)
        } else {
            (4, 0x1002)
        };
        let d = s.vm.world.objects[&target].definition.as_ref().unwrap();
        let menu = p.modules[&d.source].object_interactions[&d.object]
            .iter()
            .position(|m| m.action == bhav)
            .unwrap();
        s.enqueue(&p, 1, target, menu).map_err(anyhow::Error::msg)?;
        if activity == "sleep-pair" {
            let d = s.vm.world.objects[&23].definition.as_ref().unwrap();
            let menu = p.modules[&d.source].object_interactions[&d.object]
                .iter()
                .position(|m| m.action == 0x100a)
                .unwrap();
            s.enqueue(&p, 13, 23, menu).map_err(anyhow::Error::msg)?;
        }
    }
    let mut trace = std::fs::File::create(format!("cache/domestic-{activity}-trace.jsonl"))?;
    let mut clips = BTreeSet::new();
    for _ in 0..ticks {
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
                std::fs::write(
                    format!("cache/domestic-{activity}-failure.json"),
                    serde_json::to_vec_pretty(&Snapshot::capture(&p, &s))?,
                )?;
                anyhow::bail!(
                    "tick {} failure: {r:?}; threads {:?}",
                    s.vm.tick,
                    s.vm.threads
                        .values()
                        .filter(|t| matches!(t.status, sims_vm::Status::Faulted(_)))
                        .map(|t| &t.status)
                        .collect::<Vec<_>>()
                );
            }
            if r.unwrap().stop != StepStop::Budget {
                break;
            }
        }
        for (&id, a) in &s.animations {
            if clips.insert((id, a.clip.clone())) {
                eprintln!("tick {} #{id} clip {}", s.vm.tick, a.clip);
            }
        }
        s.vm.sounds.clear();
        s.vm.effects.clear();
        s.household.as_mut().unwrap().clock_events.clear();
    }
    std::fs::write(
        format!("cache/domestic-{activity}-checkpoint.json"),
        serde_json::to_vec_pretty(&Snapshot::capture(&p, &s))?,
    )?;
    eprintln!("Completed {} ticks; clips {clips:?}", s.vm.tick);
    Ok(())
}
