//! Private original-house rig inputs for native head-look comparisons.
use anyhow::{Context, Result};
use serde_json::json;
use sims_content::{Index, saves::House};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut rows = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        for r in &house.household.as_ref().context("household")?.residents {
            let saved = house.objects.instances[&r.object]
                .person
                .as_ref()
                .context("person")?;
            let avatar = sims_content::saved_avatar::load(&index, &r.body_strings[0], saved)?;
            rows.push(json!({
                "source":source,"object":r.object,"profile":r.body_strings,
                "floats":saved.floats,"animations":saved.animations,
                "head_fields":([41,42,43,44,45].map(|i|saved.data[i])),
                "skeleton":avatar.skeleton.name,
                "bones":avatar.skeleton.bones.iter().map(|b|json!({
                    "name":b.name,"parent":b.parent,"translation":b.translation,
                    "rotation":b.rotation,"properties":b.properties,"flags":b.flags,
                    "wiggle":b.wiggle
                })).collect::<Vec<_>>()
            }));
        }
    }
    std::fs::write(
        "cache/original-head-rigs.json",
        serde_json::to_vec_pretty(&rows)?,
    )?;
    println!("Exported {} original resident rig inputs", rows.len());
    Ok(())
}
