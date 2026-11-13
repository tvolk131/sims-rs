//! Headless original-script interaction frames with byte-identical save/resume renders.
use anyhow::{Result, bail};
use sims_content::{Index, household};
use sims_render::{Assets, Camera, room_scene};
use sims_sim::{Snapshot, StepStop};
fn main() -> Result<()> {
    let i = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = household::autonomous_household(&i)?;
    let assets = Assets::load_simulation(&i, &p, &s)?;
    let output = "cache/livable-frames";
    std::fs::create_dir_all(output)?;
    let mut done = std::collections::BTreeSet::new();
    let mut timeline = vec![];
    for tick in 1..=129600 {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        let r = s.run_fixture_tick(&p, 2000).map_err(anyhow::Error::msg)?;
        if matches!(
            r.stop,
            StepStop::Faulted | StepStop::Budget | StepStop::Animation
        ) {
            bail!("tick {tick}: {:?}", s.vm.threads);
        }
        s.vm.sounds.clear();
        s.household.as_mut().unwrap().clock_events.clear();
        let clip = s.animations.get(&1).map(|a| a.clip.as_str()).unwrap_or("");
        let label = if tick == 129600 {
            Some("day-four")
        } else if clip.contains("shower1-scrub") {
            Some("shower")
        } else if clip.contains("shower1-getout") && s.animations[&1].frame_fifths > 300 {
            Some("shower-exit")
        } else if clip.contains("bookshelf-read-sitting-loop")
            && s.vm.world.objects[&1].container.is_some()
        {
            Some("reading")
        } else if clip.contains("trashcan-trash-throwaway") {
            Some("disposal")
        } else {
            None
        };
        if let Some(label) = label.filter(|l| !done.contains(*l)) {
            let restored = Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            for close in [false, true] {
                let mut camera = Camera::room(&s, 1200, 900, 0, 0.76);
                if close {
                    camera.zoom = 2.5;
                    camera.center = glam::Vec3::new(
                        s.bodies[&1].position.x as f32 / 16.,
                        1.,
                        s.bodies[&1].position.y as f32 / 16.,
                    );
                }
                let frame =
                    sims_render::software::render(&room_scene(&assets, &p, &s, camera, 0.)?);
                assert_eq!(
                    frame,
                    sims_render::software::render(&room_scene(&assets, &p, &restored, camera, 0.)?)
                );
                frame.save(format!(
                    "{output}/{label}{}.png",
                    if close { "-detail" } else { "" }
                ))?;
            }
            std::fs::write(
                format!("{output}/{label}.json"),
                serde_json::to_vec(&Snapshot::capture(&p, &s))?,
            )?;
            timeline.push(serde_json::json!({"label":label,"tick":tick,"clip":clip}));
            done.insert(label.to_owned());
            println!("Rendered {label} at {tick}: {clip}");
        }
    }
    std::fs::write(
        format!("{output}/timeline.json"),
        serde_json::to_vec_pretty(&timeline)?,
    )?;
    if done.len() != 5 {
        bail!("missing required frames: {done:?}");
    }
    Ok(())
}
