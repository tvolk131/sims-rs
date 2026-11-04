//! Headless two-chair household diagnostics, including critical-comfort reactions.
use anyhow::{Result, bail};
use sims_content::{Index, vm::*};
use sims_render::{Assets, Camera, room_scene};
use sims_sim::{Snapshot, StepStop, clock::LotClock};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let critical = std::env::args().any(|a| a == "--critical");
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    load_chair_animations(&index, &mut p)?;
    let fixture = ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
    let mut s = initialized_two_chair_room(&p, fixture, 1)?;
    s.enable_household(
        &p,
        LotClock {
            hour: 8,
            day: 1,
            month: 1,
            year: 1997,
            month_days: 30,
            period: 0,
            minute: 0,
            second: 0,
            elapsed_days: 0,
            ticks: 0,
            statistics: [[0; 8]; 6],
        },
    )
    .map_err(anyhow::Error::msg)?;
    if critical {
        s.enable_residential_autonomy([2, 3])
            .map_err(anyhow::Error::msg)?;
        let m =
            s.vm.world
                .objects
                .get_mut(&1)
                .unwrap()
                .motives
                .as_mut()
                .unwrap();
        for i in [5, 7, 8, 9, 13, 14, 15] {
            m[i] = 50f32.to_bits();
        }
        m[6] = (-90f32).to_bits();
    } else {
        s.vm.world
            .objects
            .get_mut(&1)
            .unwrap()
            .motives
            .as_mut()
            .unwrap()[6] = (-50f32).to_bits();
        s.enqueue(&p, 1, 2, 0).map_err(anyhow::Error::msg)?;
    }
    let ticks = if critical {
        vec![
            222, 224, 225, 226, 230, 234, 258, 330, 400, 500, 600, 663, 700,
        ]
    } else {
        vec![0, 60, 150, 250, 500, 520, 540, 560, 600, 650, 700, 800, 900]
    };
    let output = if critical {
        "cache/critical-turn-frames"
    } else {
        "cache/household-frames"
    };
    std::fs::create_dir_all(output)?;
    let assets = Assets::load(&index)?;
    let mut timeline = vec![];
    for tick in 0..=*ticks.last().unwrap() {
        if tick > 0 {
            s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        }
        if !critical && tick == 500 {
            s.enqueue(&p, 1, 3, 0).map_err(anyhow::Error::msg)?;
        }
        let step = s.run_fixture_tick(&p, 10000).map_err(anyhow::Error::msg)?;
        if matches!(
            step.stop,
            StepStop::Faulted | StepStop::Animation | StepStop::Budget
        ) {
            bail!(
                "household frame scenario stopped at {tick}: {:?}",
                step.stop
            );
        }
        if !ticks.contains(&tick) {
            continue;
        }
        let camera = Camera::room(&s, 1100, 760, 0, 1.);
        let frame = sims_render::software::render(&room_scene(&assets, &p, &s, camera, 0.)?);
        frame.save(format!("{output}/t{tick:04}.png"))?;
        let restored = Snapshot::capture(&p, &s)
            .restore(&p)
            .map_err(anyhow::Error::msg)?;
        assert!(
            frame
                == sims_render::software::render(&room_scene(&assets, &p, &restored, camera, 0.)?),
            "restored frame differs at {tick}"
        );
        timeline.push(serde_json::json!({"tick":tick,"body":s.bodies[&1],"clock":s.household.as_ref().unwrap().clock,
            "motives":s.vm.world.objects[&1].motives.unwrap().map(f32::from_bits),"turn":s.turns.get(&1),"headline":s.vm.world.objects[&1].headline}));
    }
    std::fs::write(
        format!("{output}/timeline.json"),
        serde_json::to_vec_pretty(&timeline)?,
    )?;
    println!(
        "Rendered {} snapshot-identical household checkpoints",
        timeline.len()
    );
    Ok(())
}
