//! Headless stair sequence on both visible floors, including save/resume pixels.
use sims_content::Index;
use sims_render::{Assets, Camera, room_scene, software};
use sims_sim::{Snapshot, StepStop};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = sims_content::stairs::laboratory(&index)?;
    let assets = Assets::load_simulation(&index, &p, &s)?;
    std::fs::create_dir_all("cache/stair-frames")?;
    s.enqueue(&p, 1, 2, 0).map_err(anyhow::Error::msg)?;
    let mut down = false;
    for tick in 1..=1100 {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        loop {
            let r = s.run_fixture_tick(&p, 4000).map_err(anyhow::Error::msg)?;
            anyhow::ensure!(
                !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
                "tick {tick}: {:?}",
                s.vm.threads
            );
            if r.stop != StepStop::Budget {
                break;
            }
        }
        if !down
            && s.interactions[&1]
                .active
                .as_ref()
                .is_some_and(|a| a.phase == sims_sim::interaction::Phase::Seated)
        {
            s.enqueue(&p, 1, 3, 0).map_err(anyhow::Error::msg)?;
            down = true;
        }
        if [78, 80, 170, 264, 450, 560, 650, 730, 900, 1050].contains(&tick) {
            let copy = Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            for floor in [1, 2] {
                let camera = Camera::room(&s, 1000, 850, 0, 0.65).floor(floor);
                let frame = software::render(&room_scene(&assets, &p, &s, camera, 0.)?);
                assert_eq!(
                    frame,
                    software::render(&room_scene(&assets, &p, &copy, camera, 0.)?)
                );
                frame.save(format!("cache/stair-frames/tick-{tick}-floor-{floor}.png"))?;
            }
            println!(
                "{tick}: body {:?}, animation {:?}",
                s.bodies[&1],
                s.animations.get(&1).map(|a| (&a.clip, a.frame_fifths))
            );
        }
    }
    Ok(())
}
