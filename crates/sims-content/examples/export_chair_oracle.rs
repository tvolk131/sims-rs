//! Keep original-byte oracle inputs private; never commit the generated report.
use sims_content::Index;
use sims_formats::iff::Iff;
use std::{collections::BTreeMap, path::Path};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(Path::new("game-data/cabinet-extracted"))?;
    let mut modules = BTreeMap::new();
    for name in [
        "DiningChairs.iff",
        "Global.iff",
        "PersonGlobals.iff",
        r"People\TemplatePerson.iff",
    ] {
        let bytes = index.resources[index.find(name)?].read()?;
        let iff = Iff::parse(bytes)?;
        let mut chunks = BTreeMap::new();
        for c in &iff.chunks {
            if [*b"BHAV", *b"BCON", *b"OBJD"].contains(&c.tag) {
                chunks.insert(
                    format!("{}:{}", c.kind(), c.id),
                    iff.data(c)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>(),
                );
            }
        }
        modules.insert(name, chunks);
    }
    std::fs::write(
        "cache/research/sims-exe/oracle/chair-resources.json",
        serde_json::to_vec_pretty(&modules)?,
    )?;
    let p = sims_content::vm::load_program(&index, r"People\TemplatePerson.iff", "Global.iff")?;
    for (name, m) in p.modules {
        if name.contains("TemplatePerson") {
            println!(
                "{name}: strings {:?}, animation tables {:?}",
                m.strings, m.object_animation_tables
            );
        }
    }
    Ok(())
}
