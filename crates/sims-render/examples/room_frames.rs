use anyhow::Result;
use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, chair_room, load_chair_animations, load_program},
};
use sims_render::{Assets, Camera, room_scene};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let assets = Assets::load(&index)?;
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    load_chair_animations(&index, &mut program)?;
    let facing = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let fixture = ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
    let mut sim = chair_room(&program, fixture, false, facing)?;
    std::fs::create_dir_all("cache/room-frames")?;
    sim.run_fixture_tick(&program, 1000)
        .map_err(anyhow::Error::msg)?;
    let mut ticks: std::collections::BTreeSet<u64> =
        [0, 30, 80, 103, 104, 130, 150, 175, 199, 205, 206, 245].into();
    if let Some(route) = sim.routes.values().next() {
        println!(
            "distance {:.3} subtiles, arrival tick {}",
            route.locomotion.distances.last().unwrap().to_owned() as f32 / 1000.,
            route.locomotion.duration()
        );
        for p in &route.locomotion.phases {
            println!(
                "{}..{} {} ({} -> {} eighths)",
                p.start_tick,
                p.start_tick + p.ticks,
                p.clip,
                p.from_heading,
                p.to_heading
            );
            ticks.extend([
                u64::from(p.start_tick),
                u64::from(p.start_tick + p.ticks / 2),
                u64::from(p.start_tick + p.ticks),
            ]);
        }
    }
    for tick in 0..=300 {
        if tick > 0 {
            sim.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        }
        sim.run_fixture_tick(&program, 1000)
            .map_err(anyhow::Error::msg)?;
        if ticks.contains(&tick) {
            let camera = Camera::room(&sim, 1100, 760, 0, 1.);
            let scene = room_scene(&assets, &program, &sim, camera, 0.)?;
            sims_render::software::render(&scene)
                .save(format!("cache/room-frames/f{facing}-t{tick:03}.png"))?;
            println!("tick {tick}: {:?}", sim.bodies[&1]);
        }
    }
    Ok(())
}
