//! Headless routing/shoo acceptance frames, compared byte-for-byte after restore.
use anyhow::{Result, ensure};
use sims_content::{Index, domestic};
use sims_render::{Assets, Camera, room_scene};
use sims_sim::{Snapshot, StepStop};
use std::collections::BTreeSet;
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    for four in [false, true] {
        let (p, mut s) = if four {
            domestic::four_residents(&index, 43)?
        } else {
            domestic::home(&index)?
        };
        if !four {
            s.vm.world.person.as_mut().unwrap().free_will = false;
            for (id, bed) in [(1, 20), (13, 23)] {
                s.vm.world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .motives
                    .as_mut()
                    .unwrap()[5] = (-30f32).to_bits();
                let d = s.vm.world.objects[&bed].definition.as_ref().unwrap();
                let menu = p.modules[&d.source].object_interactions[&d.object]
                    .iter()
                    .position(|m| m.action == 0x100a)
                    .unwrap();
                s.enqueue(&p, id, bed, menu).map_err(anyhow::Error::msg)?;
            }
        }
        let assets = Assets::load_simulation(&index, &p, &s)?;
        let output = if four {
            "cache/routing-frames/four"
        } else {
            "cache/routing-frames/two"
        };
        std::fs::create_dir_all(output)?;
        let mut done = BTreeSet::new();
        for tick in 1..=1800 {
            s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
            loop {
                let r = s.run_fixture_tick(&p, 4000).map_err(anyhow::Error::msg)?;
                ensure!(
                    !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
                    "frame run failed at {tick}"
                );
                if r.stop != StepStop::Budget {
                    break;
                }
            }
            s.vm.sounds.clear();
            s.vm.effects.clear();
            s.household.as_mut().unwrap().clock_events.clear();
            let label = if tick == 1800 {
                Some("household")
            } else if let Some(c) = s.route_control.values().find(|c| {
                c.callback.is_some()
                    && s.animations
                        .get(&c.caller)
                        .is_some_and(|a| a.frame_fifths > 25)
            }) {
                Some(match c.callback.unwrap() {
                    sims_sim::contention::Callback::Ask => "asking",
                    sims_sim::contention::Callback::Wait => "waiting",
                })
            } else if tick > 20 && !s.routes.is_empty() {
                Some("walking")
            } else {
                None
            };
            if let Some(label) = label.filter(|l| !done.contains(*l)) {
                let restored = Snapshot::capture(&p, &s)
                    .restore(&p)
                    .map_err(anyhow::Error::msg)?;
                for rotation in 0..4 {
                    let camera = Camera::room(&s, 1400, 1000, rotation, 0.76);
                    let frame =
                        sims_render::software::render(&room_scene(&assets, &p, &s, camera, 0.5)?);
                    ensure!(
                        frame
                            == sims_render::software::render(&room_scene(
                                &assets, &p, &restored, camera, 0.5
                            )?),
                        "render changed after restore"
                    );
                    frame.save(format!("{output}/{label}-r{rotation}.png"))?;
                }
                std::fs::write(
                    format!("{output}/{label}.json"),
                    serde_json::to_vec_pretty(&Snapshot::capture(&p, &s))?,
                )?;
                println!("{output}/{label}: tick {tick}");
                done.insert(label);
            }
        }
        if !four {
            ensure!(
                done.contains("asking") && done.contains("waiting"),
                "missing contention frames: {done:?}"
            );
        }
    }
    Ok(())
}
