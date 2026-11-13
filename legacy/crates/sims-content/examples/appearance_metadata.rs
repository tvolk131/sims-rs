//! Private read-only inspection of an original appearance catalog and its meshes.
use anyhow::{Context, Result};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut name = args.next().context("provide a BCF resource name")?;
    let appearance_name = name == "--appearance";
    if appearance_name {
        name = args.next().context("provide an appearance name")?;
    }
    let index = sims_content::Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let id = if appearance_name {
        sims_content::avatar::find_appearance_catalog(&index, &name)?
    } else {
        index.find(&name)?
    };
    let resource = &index.resources[id];
    let catalog = sims_formats::avatar::read_bcf(&resource.read()?)?;
    for appearance in catalog
        .appearances
        .into_iter()
        .filter(|a| !appearance_name || a.name.eq_ignore_ascii_case(&name))
    {
        for binding in appearance.bindings {
            let r = &index.resources[index.find(&format!("{}.bmf", binding.mesh))?];
            let mesh = sims_formats::avatar::read_bmf(&r.read()?)?;
            println!(
                "{}",
                serde_json::json!({"catalog":resource.locator,"appearance":appearance.name,"kind":appearance.kind,"bone":binding.bone,"censor":binding.censor,"mesh":binding.mesh,"texture":mesh.texture,"bones":mesh.bones,"vertices":mesh.vertices.len()})
            );
        }
    }
    Ok(())
}
