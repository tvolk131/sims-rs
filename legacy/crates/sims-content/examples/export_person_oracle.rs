//! Private synthetic-world inputs plus extracted resources for native Main replay.
use sims_content::{Index, vm::*};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    p.modules
        .extend(load_program(&index, r"People\TemplatePerson.iff", "Global.iff")?.modules);
    let f = ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
    let s = two_chair_room(&p, f, 1)?;
    std::fs::write(
        "cache/research/sims-exe/oracle/person-world.json",
        serde_json::to_vec_pretty(&s.vm.world)?,
    )?;
    Ok(())
}
