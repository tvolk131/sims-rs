//! Private decoded original animation inputs for the native CPU sampler oracle.
use anyhow::{Context, Result};
use serde_json::json;
use sims_content::{Index, saves::House};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut program = sims_vm::Program::default();
    let mut rows = vec![];
    let mut layer_rows = vec![];
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
            let layers = sims_content::saved_layers::load(
                &index,
                &mut program,
                &avatar.skeleton,
                saved,
                123456,
                1,
            )?;
            for (slot, layer) in layers.into_iter().enumerate() {
                if let Some(layer) = layer {
                    layer_rows.push(json!({"source":source,"object":r.object,"slot":slot,
                        "descriptor":saved.animations[slot],"layer":layer}));
                }
            }
            for name in saved
                .animations
                .iter()
                .filter_map(|s| s.split(';').next())
                .filter(|s| !s.is_empty())
            {
                sims_content::assets::load_named_animations(
                    &index,
                    &mut program,
                    &[name.to_owned()].into(),
                )?;
                rows.push(json!({"source":source,"object":r.object,"skeleton":avatar.skeleton.name,
                    "animation":program.animations[name],
                    "bones":avatar.skeleton.bones.iter().map(|b|json!({"name":b.name,"parent":b.parent,
                        "translation":b.translation,"rotation":b.rotation,"flags":b.flags})).collect::<Vec<_>>() }));
            }
        }
    }
    std::fs::write(
        "cache/original-pose-inputs.json",
        serde_json::to_vec(&rows)?,
    )?;
    println!("Exported {} original resident/clip inputs", rows.len());
    std::fs::write(
        "cache/original-layer-inputs.json",
        serde_json::to_vec(&layer_rows)?,
    )?;
    println!("Exported {} bound original saved layers", layer_rows.len());
    Ok(())
}
