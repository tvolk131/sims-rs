//! Export original household resources to ignored private research storage.
use sims_content::Index;
use sims_formats::{iff::Iff, strings::StringTable};
use std::{collections::BTreeMap, path::Path};

fn main() -> anyhow::Result<()> {
    let index = Index::scan(Path::new("game-data/cabinet-extracted"))?;
    let mut modules = BTreeMap::new();
    let mut strings = BTreeMap::new();
    for name in [
        "Global.iff",
        "PersonGlobals.iff",
        r"People\TemplatePerson.iff",
        "DiningChairs.iff",
        "Beds.iff",
        "Fridges.iff",
        "Food.iff",
        "Recliners.iff",
        "Sinks.iff",
        "ShowerC.iff",
        "Toilets.iff",
    ] {
        let iff = Iff::parse(index.resources[index.find(name)?].read()?)?;
        let mut chunks = BTreeMap::new();
        for c in &iff.chunks {
            if [
                *b"BHAV", *b"BCON", *b"OBJD", *b"OBJf", *b"FCNS", *b"SLOT", *b"STR#", *b"TTAB",
            ]
            .contains(&c.tag)
            {
                chunks.insert(
                    format!("{}:{}", c.kind(), c.id),
                    iff.data(c)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>(),
                );
            }
            if c.tag == *b"STR#" {
                match StringTable::parse(iff.data(c)) {
                    Ok(table) => {
                        strings.insert(
                            format!("{name}/{} {}", c.id, c.label),
                            table
                                .entries
                                .iter()
                                .filter(|e| e.language == 1)
                                .map(|e| e.text.clone())
                                .collect::<Vec<_>>(),
                        );
                    }
                    Err(error) => eprintln!(
                        "{name} STR#{} {}: {error}; retained raw bytes",
                        c.id, c.label
                    ),
                }
            }
        }
        println!("{name}: {} research resources", chunks.len());
        modules.insert(name, chunks);
    }
    std::fs::write(
        "cache/research/sims-exe/oracle/household-resources.json",
        serde_json::to_vec_pretty(&modules)?,
    )?;
    std::fs::write(
        "cache/research/sims-exe/oracle/household-strings.json",
        serde_json::to_vec_pretty(&strings)?,
    )?;
    Ok(())
}
