//! Resume a private diagnostic snapshot without rerunning days of setup.
use sims_content::{Index, domestic};
use sims_sim::{Snapshot, StepStop};
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (p, _) = if args.get(2).is_some_and(|a| a == "--home") {
        sims_content::household::two_sim_home(&index)?
    } else {
        domestic::home_with(&index, 7, domestic::Layout::StraightKitchen)?
    };
    let mut s = Snapshot::from_json(&std::fs::read(&args[0])?, &p).map_err(anyhow::Error::msg)?;
    let ticks: usize = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(1000);
    for n in 0..ticks {
        if n != 0 {
            s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
        }
        let mut finished = false;
        for _ in 0..100 {
            let r = s.run_fixture_tick(&p, 4000).map_err(anyhow::Error::msg)?;
            anyhow::ensure!(
                !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
                "tick {}: {:?}",
                s.vm.tick,
                r
            );
            if r.stop != StepStop::Budget {
                finished = true;
                break;
            }
        }
        anyhow::ensure!(finished, "stalled tick {}", s.vm.tick);
        s.vm.sounds.clear();
        s.vm.effects.clear();
        s.household.as_mut().unwrap().clock_events.clear();
    }
    println!("Resumed through tick {}", s.vm.tick);
    Ok(())
}
