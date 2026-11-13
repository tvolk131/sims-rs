//! Render verified needs-object snapshots without starting a window or GUI app.
use anyhow::Result;
use sims_content::Index;
use sims_render::{Assets, Camera, room_scene};
use sims_sim::Snapshot;
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let sink = std::env::args().any(|a| a == "--sink");
    let fridge = std::env::args().any(|a| a == "--fridge");
    let recliner = std::env::args().any(|a| a == "--recliner");
    let (selector, motive, label) = if recliner {
        ("Recliners.iff", 5, "recliner")
    } else if fridge {
        ("Fridges.iff", 7, "fridge")
    } else if sink {
        ("Sinks.iff", 8, "sink")
    } else {
        ("Toilets.iff", 9, "toilet")
    };
    let (p, _) = sims_content::household::needs_room(&index, selector, 16807, motive)?;
    let mut objects = vec![("DiningChairs.iff", 16807), (selector, 16807)];
    if recliner {
        objects = vec![
            ("DiningChairs.iff", 16807),
            (selector, 16808),
            (selector, 16809),
        ];
    }
    if fridge {
        objects.extend([("Food.iff", 16808), ("trashpile.iff", 16807)]);
    }
    let assets = Assets::load_objects(&index, &objects)?;
    let input = format!("cache/research/sims-exe/household-object/{selector}-16807-{motive}");
    let closeup = std::env::args().any(|a| a == "--closeup");
    let output = format!(
        "cache/{label}-frames{}",
        if closeup { "-closeup" } else { "" }
    );
    std::fs::create_dir_all(&output)?;
    let mut timeline = vec![];
    let frames = if recliner {
        vec![
            "tick-100",
            "tick-510",
            "tick-620",
            "tick-1000",
            "tick-4000",
            "tick-5000",
            "tick-6000",
            "tick-8000",
            "tick-10000",
            "tick-12000",
            "completed",
        ]
    } else {
        vec![
            "tick-100",
            "tick-490",
            "tick-510",
            "tick-535",
            "tick-555",
            "tick-620",
            "tick-766",
            "tick-900",
            "tick-1060",
            "completed",
        ]
    };
    for name in frames {
        let bytes = std::fs::read(format!("{input}/{name}.json"))?;
        let sim = Snapshot::from_json(&bytes, &p).map_err(anyhow::Error::msg)?;
        let mut camera = Camera::room(&sim, 1100, 760, if fridge { 2 } else { 0 }, 1.);
        if closeup {
            camera.zoom = 3.;
            camera.center = glam::Vec3::new(
                sim.bodies[&1].position.x as f32 / 16.,
                1.,
                sim.bodies[&1].position.y as f32 / 16.,
            );
        }
        let frame = sims_render::software::render(&room_scene(&assets, &p, &sim, camera, 0.)?);
        frame.save(format!("{output}/{name}.png"))?;
        let restored = Snapshot::capture(&p, &sim)
            .restore(&p)
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            frame,
            sims_render::software::render(&room_scene(&assets, &p, &restored, camera, 0.)?)
        );
        timeline.push(serde_json::json!({"tick":sim.vm.tick,"body":sim.bodies[&1],"object_graphic":sim.vm.world.objects[&4].data[&0],"animation":sim.animations.get(&1),"motive":motive,"value":f32::from_bits(sim.vm.world.objects[&1].motives.unwrap()[motive])}));
    }
    std::fs::write(
        format!("{output}/timeline.json"),
        serde_json::to_vec_pretty(&timeline)?,
    )?;
    println!(
        "Rendered {} snapshot-identical {label} interaction frames headlessly",
        timeline.len()
    );
    Ok(())
}
