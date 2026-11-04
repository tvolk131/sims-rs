//! Headless original-script interaction frames with byte-identical save/resume renders.
use anyhow::{Result, bail};
use sims_content::Index;
use sims_render::{Assets, Camera, room_scene};
use sims_sim::{Snapshot, StepStop};
fn main() -> Result<()> {
    let i = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, mut s) = sims_content::domestic::home(&i)?;
    let assets = Assets::load_simulation(&i, &p, &s)?;
    let output = "cache/domestic-frames";
    std::fs::create_dir_all(output)?;
    let mut done = std::collections::BTreeSet::new();
    let mut timeline = vec![];
    for tick in 1..=43200 {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        let r = s.run_fixture_tick(&p, 2000).map_err(anyhow::Error::msg)?;
        if matches!(
            r.stop,
            StepStop::Faulted | StepStop::Budget | StepStop::Animation
        ) {
            bail!("tick {tick}: {:?}", s.vm.threads);
        }
        s.vm.sounds.clear();
        s.vm.effects.clear();
        s.household.as_mut().unwrap().clock_events.clear();
        let clip = s.animations.get(&1).map(|a| a.clip.as_str()).unwrap_or("");
        let label = if tick == 43200 {
            Some("next-morning")
        } else if clip.contains("counter-foodprep") {
            Some("preparation")
        } else if clip.contains("stove-cook") {
            Some("cooking")
        } else if clip.contains("beds-sleep") {
            Some("sleeping")
        } else if clip.contains("sit-scoot-stabfood-lift") {
            Some("dining")
        } else if clip.contains("sink-washdishes") {
            Some("washing")
        } else {
            None
        };
        let ready = s.animations.get(&1).is_some_and(|a| {
            a.frame_fifths >= 100.min(u64::from(p.animations[&a.clip].clip.frames) * 5 / 2)
        });
        if let Some(label) = label.filter(|l| !done.contains(*l) && (ready || *l == "next-morning"))
        {
            let restored = Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            for rotation in 0..4 {
                for close in [false, true] {
                    let mut camera = Camera::room(&s, 1200, 900, rotation, 0.76);
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
                        sims_render::software::render(&room_scene(
                            &assets, &p, &restored, camera, 0.
                        )?)
                    );
                    frame.save(format!(
                        "{output}/{label}{}{}.png",
                        if close { "-detail" } else { "" },
                        if rotation == 0 {
                            String::new()
                        } else {
                            format!("-r{rotation}")
                        }
                    ))?;
                }
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
    if done.len() != 6 {
        bail!("missing required frames: {done:?}");
    }
    Ok(())
}
