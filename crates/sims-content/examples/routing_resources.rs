//! Read-only private content inspection for native routing/stair research.
use sims_content::{Index, assets, vm};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut p = vm::load_program(&index, "PersonGlobals.iff", "Global.iff")?;
    assets::load_object_guids(&index, &mut p, &[0x7c4])?;
    for m in p.modules.values().filter(|m| {
        m.object_definition_words
            .values()
            .any(|w| w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16) == 0x7c4)
    }) {
        println!("GOHERE {}", m.source);
        for (id, r) in &m.routines {
            println!("{id}: {}\n{}", r.name, r.body.disassemble(|_| None));
        }
        println!(
            "Menus {:?}\nFunctions {:?}",
            m.object_interactions, m.object_functions
        );
    }
    for file in ["stair.iff", "Doors.iff", "PersonGlobals.iff", "Global.iff"] {
        let d = index.document(index.find(file)?)?;
        for b in d.behaviors.iter().filter(|b| {
            file == "Doors.iff"
                || file == "stair.iff"
                || [281, 8276].contains(&b.id)
                || b.name.to_lowercase().contains("route")
        }) {
            println!(
                "{file} {} {}\n{}",
                b.id,
                b.name,
                b.value.disassemble(|id| d.behavior_name(id))
            );
        }
    }
    Ok(())
}
