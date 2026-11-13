//! Inspect private resources relevant to the original person/action dispatcher.
use sims_content::Index;
use sims_formats::{
    behavior::{Behavior, InteractionTable},
    iff::Iff,
};
fn main() -> anyhow::Result<()> {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "game-data/cabinet-extracted".into());
    let index = Index::scan(&root)?;
    for name in [
        "Global.iff",
        "PersonGlobals.iff",
        r"People\TemplatePerson.iff",
        "DiningChairs.iff",
    ] {
        let res = &index.resources[index.find(name)?];
        let iff = Iff::parse(res.read()?)?;
        println!("{}", res.locator);
        for chunk in &iff.chunks {
            if chunk.tag == *b"TTAB" {
                let table = InteractionTable::parse(iff.data(chunk))?;
                println!("  TTAB {}: {} entries", chunk.id, table.interactions.len());
                for entry in table
                    .interactions
                    .iter()
                    .filter(|e| e.string_index == 33 || name == "DiningChairs.iff")
                {
                    println!(
                        "    identifier {} action {} check {} flags {:x}",
                        entry.string_index, entry.action, entry.test, entry.flags
                    );
                }
            }
            if chunk.tag == *b"BHAV" {
                let b = Behavior::parse(iff.data(chunk))?;
                if b.instructions.iter().any(|i| i.opcode == 17) {
                    println!(
                        "  idle BHAV {} {}: {:?}",
                        chunk.id, chunk.label, b.instructions
                    );
                }
            }
        }
    }
    Ok(())
}
