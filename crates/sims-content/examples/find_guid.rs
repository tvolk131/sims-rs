use anyhow::Result;
use sims_content::Index;
fn main() -> Result<()> {
    let guid = u32::from_str_radix(&std::env::args().nth(1).unwrap(), 16)?;
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    for (id, r) in index
        .resources
        .iter()
        .enumerate()
        .filter(|(_, r)| r.is_iff())
    {
        if let Ok(d) = index.document(id) {
            for o in d.objects {
                if o.value.guid == guid {
                    println!("Matched {} OBJD {}", r.locator, o.id);
                    let p = sims_content::vm::load_program(&index, &r.locator, "Global.iff")?;
                    for (&id, w) in &p.modules[&r.locator].object_definition_words {
                        println!(
                            "{} #{id}: type={} group={} part={} master={}",
                            r.locator,
                            w[9],
                            w[10],
                            w[11],
                            w[0x66 / 2]
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
