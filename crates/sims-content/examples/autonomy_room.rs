//! Original Init/Main and chair autonomy on a supplied residential test lot.
use anyhow::{Result, bail};
use sims_content::{Index, vm::*};
use sims_sim::{StepStop, clock::LotClock};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    load_chair_animations(&index, &mut p)?;
    let fixture = ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
    let mut s = initialized_two_chair_room(&p, fixture, 1)?;
    println!(
        "original Init autonomy level {}",
        s.vm.world.objects[&1].person.as_ref().unwrap()[&36]
    );
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
    s.enable_residential_autonomy([2, 3])
        .map_err(anyhow::Error::msg)?;
    let motives =
        s.vm.world
            .objects
            .get_mut(&1)
            .unwrap()
            .motives
            .as_mut()
            .unwrap();
    for index in [5, 7, 8, 9, 13, 14, 15] {
        motives[index] = 50f32.to_bits();
    }
    motives[6] = if std::env::args().any(|s| s == "--critical") {
        -90f32
    } else {
        -30f32
    }
    .to_bits();
    let mut trace = vec![];
    let mut searches = 0;
    for _ in 0..1200 {
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        let step = s.run_fixture_tick(&p, 10000).map_err(anyhow::Error::msg)?;
        searches += step.trace.iter().filter(|t| t.opcode == 3).count();
        trace.extend(step.trace);
        if matches!(
            step.stop,
            StepStop::Faulted | StepStop::Animation | StepStop::Budget
        ) {
            std::fs::write(
                "cache/research/sims-exe/autonomy-room-trace.json",
                serde_json::to_vec_pretty(&trace)?,
            )?;
            bail!("stopped {:?}: {:?}", step.stop, s.vm.threads);
        }
        if s.vm.world.objects[&1].container.is_some() {
            println!(
                "autonomously contained at tick {}, searches {searches}",
                s.vm.tick
            );
            break;
        }
    }
    std::fs::write(
        "cache/research/sims-exe/autonomy-room-trace.json",
        serde_json::to_vec_pretty(&trace)?,
    )?;
    std::fs::write(
        "cache/research/sims-exe/autonomy-room.json",
        serde_json::to_vec_pretty(&sims_sim::Snapshot::capture(&p, &s))?,
    )?;
    if s.vm.world.objects[&1].container.is_none() {
        bail!("autonomy never entered a chair");
    }
    sims_sim::Snapshot::capture(&p, &s)
        .restore(&p)
        .map_err(anyhow::Error::msg)?;
    println!("searches {searches}; snapshot validated");
    Ok(())
}
