//! Private-content diagnostic for the original adult Main path.
use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, load_chair_animations, load_program, two_chair_room},
};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    load_chair_animations(&index, &mut p)?;
    println!(
        "idle strings {:?}",
        p.modules[&p.globals].strings.get(&156).map(|s| &s[15..18])
    );
    let fixture = ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
    let mut s = two_chair_room(&p, fixture, 1)?;
    let t = s.interactions[&1].main.unwrap();
    let actions = std::env::args().any(|a| a == "--actions");
    for _ in 0..2000 {
        if actions && s.vm.tick == 5 {
            s.enqueue(&p, 1, 2, 0).map_err(anyhow::Error::msg)?;
        }
        if actions && s.vm.tick == 500 {
            s.enqueue(&p, 1, 3, 0).map_err(anyhow::Error::msg)?;
        }
        if actions && s.vm.tick == 1100 {
            if let Some(a) = s.interactions[&1].active.clone() {
                s.cancel_interaction(&p, 1, a.item.id)
                    .map_err(anyhow::Error::msg)?;
            }
        }
        let run = s.run_fixture_tick(&p, 1000).map_err(anyhow::Error::msg)?;
        println!("tick {} {:?}", s.vm.tick, run);
        if matches!(
            run.stop,
            sims_sim::StepStop::Faulted | sims_sim::StepStop::Animation
        ) {
            break;
        }
        if matches!(run.stop, sims_sim::StepStop::Budget) {
            continue;
        }
        if s.vm.tick % 200 == 0 {
            let restored = sims_sim::Snapshot::capture(&p, &s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            assert_eq!(s, restored);
        }
        s.advance_fixture_tick().map_err(anyhow::Error::msg)?;
    }
    println!("thread {:#?}", s.vm.threads[&t]);
    println!("queue {:?}", s.interactions);
    Ok(())
}
