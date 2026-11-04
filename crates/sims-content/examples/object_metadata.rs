//! Read-only private metadata export for native factory research.
use anyhow::Result;
fn main() -> Result<()> {
    let index = sims_content::Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let selector = std::env::args().nth(1).expect("resource selector");
    let doc = index.document(index.find(&selector)?)?;
    if std::env::args().any(|a| a == "--strings") {
        for chunk in doc.iff.chunks.iter().filter(|c| c.tag == *b"STR#") {
            let Ok(strings) = sims_formats::strings::StringTable::parse(doc.iff.data(chunk)) else {
                continue;
            };
            let entries: Vec<_> = strings
                .entries
                .iter()
                .filter(|e| e.language == 1)
                .map(|e| e.text.clone())
                .collect();
            println!("{}", serde_json::json!({"id":chunk.id,"entries":entries}));
        }
        return Ok(());
    }
    let program = sims_content::vm::load_program(&index, &selector, "Global.iff")?;
    let module = &program.modules[&doc.source];
    let objects:Vec<_>=doc.objects.iter().map(|o|serde_json::json!({"id":o.id,"definition":format!("{:?}",o.value),"words":module.object_definition_words[&o.id],"functions":module.object_functions[&o.id],"slots":module.slots.get(&o.value.slots)})).collect();
    println!("{}", serde_json::to_string_pretty(&objects)?);
    Ok(())
}
