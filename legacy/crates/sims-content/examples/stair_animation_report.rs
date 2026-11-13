//! Read-only original stair clip/root samples for floor-transition presentation.
fn main() -> anyhow::Result<()> {
    let index = sims_content::Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut p = sims_content::vm::load_program(&index, "stair.iff", "Global.iff")?;
    sims_content::assets::load_linked_animations(&index, &mut p)?;
    for name in ["a2o-stair-climb", "a2o-stair-descend"] {
        let a = &p.animations[name];
        println!("{name}: {} frames", a.clip.frames);
        for m in &a.clip.motions {
            if let Some(start) = m.translation {
                println!(
                    "{} {:?}",
                    m.bone,
                    [0, m.frames / 2, m.frames - 1]
                        .map(|i| a.samples.translations[(start + i) as usize])
                );
            }
        }
    }
    Ok(())
}
