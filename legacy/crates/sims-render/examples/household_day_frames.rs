//! Software-rendered checkpoints from the autonomous household diagnostic.
use anyhow::{Context, Result};
use sims_content::{Index, household};
use sims_render::{Assets, Camera, room_scene};
use sims_sim::Snapshot;
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, _) = household::autonomous_household(&index)?;
    let assets = Assets::load_household(&index)?;
    let output = "cache/household-day-frames";
    std::fs::create_dir_all(output)?;
    let mut visible_accessory_frames = 0;
    for tick in [778, 800, 900, 1000, 1100, 4200, 4400, 18000, 26000, 40000] {
        let bytes = std::fs::read(format!(
            "cache/research/sims-exe/household-day/tick-{tick}.json"
        ))?;
        let sim = Snapshot::from_json(&bytes, &p)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("checkpoint {tick}"))?;
        for closeup in [false, true] {
            let mut camera = Camera::room(&sim, 1100, 760, 0, 1.);
            if closeup {
                camera.zoom = 3.;
                camera.center = glam::Vec3::new(
                    sim.bodies[&1].position.x as f32 / 16.,
                    1.,
                    sim.bodies[&1].position.y as f32 / 16.,
                );
            }
            let scene = room_scene(&assets, &p, &sim, camera, 0.)?;
            let frame = sims_render::software::render(&scene);
            frame.save(format!(
                "{output}/tick-{tick}{}.png",
                if closeup { "-closeup" } else { "" }
            ))?;
            let restored = Snapshot::capture(&p, &sim)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            assert!(
                frame
                    == sims_render::software::render(&room_scene(
                        &assets, &p, &restored, camera, 0.
                    )?),
                "restored frame differs at {tick}"
            );
            if !sim.vm.world.objects[&1].accessories.is_empty() {
                let mut removed = sim.clone();
                removed
                    .vm
                    .world
                    .objects
                    .get_mut(&1)
                    .unwrap()
                    .accessories
                    .clear();
                let without = room_scene(&assets, &p, &removed, camera, 0.)?;
                assert!(scene.vertices.len() > without.vertices.len());
                if closeup && frame != sims_render::software::render(&without) {
                    visible_accessory_frames += 1;
                }
            }
        }
    }
    assert!(
        visible_accessory_frames > 0,
        "accessories were never visible"
    );
    println!("Rendered 20 snapshot-identical household frames, including accessory geometry");
    Ok(())
}
