//! Read-only inventory of the original indicator definitions and avatar assets.
use anyhow::Result;
use serde_json::json;
use sims_content::Index;
use sims_formats::{iff::Iff, strings::StringTable};

fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut objects = vec![];
    for name in ["PlumbBob.iff", "DateBob.iff", "StarBob.iff"] {
        let resource = &index.resources[index.find(name)?];
        let program = sims_content::vm::load_program(&index, name, "Global.iff")?;
        let module = &program.modules[&resource.locator];
        let iff = Iff::parse(resource.read()?)?;
        let mut tables = std::collections::BTreeMap::new();
        for chunk in iff.all(b"STR#") {
            tables.insert(
                chunk.id,
                StringTable::parse(iff.data(chunk))?
                    .entries
                    .into_iter()
                    .filter(|entry| entry.language == 1)
                    .map(|entry| entry.text)
                    .collect::<Vec<_>>(),
            );
        }
        let definitions = module.object_definition_words.iter().map(|(&id, words)| {
            json!({"id":id,"kind":words[9],"profile_table":words[19],"native_cc":words[102]})
        }).collect::<Vec<_>>();
        println!(
            "{}: {}",
            resource.locator,
            json!({"definitions":definitions,"strings":tables})
        );
        objects.push(json!({"source":resource.locator,"definitions":definitions,"strings":tables}));
    }
    let assets = index.resources.iter().filter(|r| {
        let name = r.name.to_ascii_lowercase();
        ["head-arrow", "head-star", "head_arrow", "head_star"].iter().any(|s| name.contains(s))
    }).map(|resource| {
        let data = resource.read()?;
        let mut value = json!({"source":resource.locator,"bytes":data.len()});
        if resource.name.to_ascii_lowercase().ends_with(".cmx.bcf") {
            use sims_formats::avatar::{catalog::{Asset, Binary}, index::Kind};
            let mut catalog = Binary::new(&data);
            let mut records = vec![];
            for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
                let count = catalog.count(kind)?.ok_or_else(|| anyhow::anyhow!("short catalog"))?;
                for _ in 0..count {
                    records.push(match catalog.read(kind)? {
                        Asset::Skeleton(s) => json!({"skeleton":s}),
                        Asset::Appearance(a) => json!({"appearance":a.name,"kind":a.kind,"bindings":a.bindings.iter().map(|b| {
                            json!({"bone":b.bone,"mesh":b.mesh,"censor":b.censor})
                        }).collect::<Vec<_>>()}),
                        Asset::Animation(a) => json!({"animation":a.name}),
                    });
                }
            }
            value["records"] = json!(records);
        }
        Ok(value)
    }).collect::<Result<Vec<_>>>()?;
    std::fs::write(
        "cache/research/sims-exe/oracle/indicator-original-assets.json",
        serde_json::to_vec_pretty(&json!({"objects":objects,"assets":assets}))?,
    )?;
    Ok(())
}
