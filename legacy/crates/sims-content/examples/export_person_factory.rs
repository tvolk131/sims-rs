//! Original NPC resources for offline factory comparison; output stays private.
use sims_content::{Index, vm::load_program};
use sims_formats::iff::Iff;
use std::{collections::BTreeMap, io::Write, path::Path};

fn main() -> anyhow::Result<()> {
    let index = Index::scan(Path::new("game-data/cabinet-extracted"))?;
    let program = load_program(&index, "NPC_TravelingSalesman.iff", "Global.iff")?;
    let (private, module) = program
        .modules
        .iter()
        .find(|(_, m)| {
            m.object_definition_words.values().any(|w| {
                w.len() > 15
                    && (u32::from(w[14] as u16) | u32::from(w[15] as u16) << 16) == 0x44a86a6c
            })
        })
        .ok_or_else(|| anyhow::anyhow!("Mystery Man definition missing"))?;
    let mut modules = BTreeMap::new();
    for source in program.modules.keys() {
        let iff = Iff::parse(index.resources[index.find(source)?].read()?)?;
        let chunks: BTreeMap<_, _> = iff
            .chunks
            .iter()
            .filter(|c| [*b"BHAV", *b"BCON", *b"OBJD", *b"OBJf", *b"SLOT"].contains(&c.tag))
            .map(|c| {
                (
                    format!("{}:{}", c.kind(), c.id),
                    iff.data(c)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>(),
                )
            })
            .collect();
        modules.insert(source, chunks);
    }
    let report = serde_json::json!({"private":private,"semiglobal":module.semiglobal,
        "global":program.globals,"modules":modules,"program":program});
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "cache/research/sims-exe/oracle/person-factory-resources.json".into());
    // Retained native results bind to the previous export's exact bytes.
    // A new inspection must never overwrite their input evidence.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?
        .write_all(&serde_json::to_vec(&report)?)?;
    println!(
        "{} / {:?}: {} modules exported",
        private,
        module.semiglobal,
        program.modules.len()
    );
    Ok(())
}
