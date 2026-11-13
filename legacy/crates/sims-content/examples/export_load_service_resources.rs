//! Private evidence for the remaining original-home loading services.
use serde_json::json;
use sims_content::{Index, assets, vm::load_program};
use sims_formats::iff::Iff;
use std::{collections::BTreeMap, path::Path};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(Path::new("game-data/cabinet-extracted"))?;
    let work = Iff::parse(index.resources[index.find("work.iff")?].read()?)?;
    println!(
        "work.iff resource-map offset: {}",
        u32::from_be_bytes(work.bytes[60..64].try_into()?)
    );
    for c in work.all(b"rsmp") {
        std::fs::write(
            "cache/research/sims-exe/oracle/work-resource-map.bin",
            work.data(c),
        )?;
        println!("work resource map prefix: {:02x?}", &work.data(c)[..48]);
    }
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    assets::load_object_guids(&index, &mut p, &[0xfd4fb8e4])?;
    let mut modules = BTreeMap::new();
    let mut definitions = vec![];
    for m in p.modules.values() {
        let iff = Iff::parse(index.resources[index.find(&m.source)?].read()?)?;
        for (&id, w) in &m.object_definition_words {
            let guid = u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16);
            if guid == 0xfd4fb8e4 {
                definitions.push(json!({"source":m.source,"id":id,"guid":guid,"words":w,
                    "functions":m.object_functions[&id],"label":iff.get(b"OBJD",id).unwrap().label}));
            }
        }
        let mut chunks = BTreeMap::new();
        for c in &iff.chunks {
            if [*b"BHAV", *b"BCON", *b"OBJD", *b"OBJf", *b"SLOT"].contains(&c.tag) {
                chunks.insert(
                    format!("{}:{}", c.kind(), c.id),
                    iff.data(c)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>(),
                );
            }
        }
        modules.insert(m.source.clone(), chunks);
    }
    std::fs::write(
        "cache/research/sims-exe/oracle/load-service-resources.json",
        serde_json::to_vec_pretty(&json!({"definitions":definitions,"modules":modules}))?,
    )?;
    println!("{}", serde_json::to_string_pretty(&definitions)?);
    Ok(())
}
