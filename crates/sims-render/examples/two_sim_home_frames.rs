//! Headless checks of architecture, independent avatars, portal and social poses.
use anyhow::{Result, bail};
use sims_content::{Index, household};
use sims_render::{Assets, Camera, room_scene};
use sims_sim::{Snapshot, StepStop};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, initial) = household::two_sim_home(&index)?;
    let assets = Assets::load_simulation(&index, &p, &initial)?;
    let out = "cache/two-sim-home-frames";
    std::fs::create_dir_all(out)?;
    // Optional saved long-run states exercise the same renderer and restore
    // contract without replaying tens of thousands of ticks for each view.
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path)?;
        let s = Snapshot::from_json(&bytes, &p).map_err(anyhow::Error::msg)?;
        let restored = Snapshot::capture(&p, &s)
            .restore(&p)
            .map_err(anyhow::Error::msg)?;
        let name = std::path::Path::new(&path)
            .file_stem()
            .unwrap()
            .to_string_lossy();
        for rotation in 0..4 {
            let camera = Camera::room(&s, 1100, 760, rotation, 0.6);
            let scene = room_scene(&assets, &p, &s, camera, 0.)?;
            for id in [1, 13] {
                assert!(scene.draws.iter().any(|d| d.object == Some(id)));
            }
            let image = sims_render::software::render(&scene);
            assert!(
                image
                    == sims_render::software::render(&room_scene(
                        &assets, &p, &restored, camera, 0.
                    )?)
            );
            image.save(format!("{out}/{name}-r{rotation}.png"))?;
        }
        println!("Rendered four snapshot-identical rotations: {path}");
    }
    let mut count = 0;
    for social in [false, true] {
        let mut s = initial.clone();
        if social {
            s.vm.world.person.as_mut().unwrap().free_will = false;
            let d = s.vm.world.objects[&13].definition.as_ref().unwrap();
            let n = p.modules[&d.source].object_interactions[&d.object]
                .iter()
                .position(|e| e.action == 0x2051)
                .unwrap();
            s.enqueue(&p, 1, 13, n).map_err(anyhow::Error::msg)?;
        }
        for tick in 0..=2400 {
            if tick > 0 {
                s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
                let r = s.run_fixture_tick(&p, 10000).map_err(anyhow::Error::msg)?;
                if matches!(
                    r.stop,
                    StepStop::Faulted | StepStop::Animation | StepStop::Budget
                ) {
                    bail!("frame scenario tick {tick}: {:?}", s.vm.threads);
                }
            }
            if ![0, 150, 400, 800, 1400, 2400].contains(&tick) {
                continue;
            }
            let restored = Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            for rotation in if tick == 0 { vec![0, 1, 2, 3] } else { vec![0] } {
                let camera = Camera::room(&s, 1100, 760, rotation, 0.6);
                let scene = room_scene(&assets, &p, &s, camera, 0.)?;
                for id in [1, 13] {
                    assert!(
                        scene.draws.iter().any(|d| d.object == Some(id)),
                        "resident missing from scene"
                    );
                }
                let image = sims_render::software::render(&scene);
                assert!(
                    image
                        == sims_render::software::render(&room_scene(
                            &assets, &p, &restored, camera, 0.
                        )?),
                    "restored pixels differ"
                );
                let name = format!(
                    "{out}/{}-{tick:04}-r{rotation}",
                    if social { "talk" } else { "home" }
                );
                image.save(format!("{name}.png"))?;
                std::fs::write(
                    format!("{name}.json"),
                    serde_json::to_vec(&Snapshot::capture(&p, &s))?,
                )?;
                count += 1;
            }
        }
    }
    println!("Rendered {count} snapshot-identical two-resident frames");
    Ok(())
}
