//! Resource resolution diagnostics without modifying original content.
use anyhow::Result;
use sims_content::{Index, household};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let (mut p, _) = household::autonomous_household(&index)?;
    // Re-resolve the linked names, retaining the verified locomotion provider choices.
    let loaded = p.animations.len();
    let report = sims_content::assets::load_linked_animations(&index, &mut p)?;
    let (_, accessories) = sims_content::avatar::load_linked_accessories_report(&index, &p)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"linked_animation_count":loaded,"animations":report,"accessories":accessories})
        )?
    );
    Ok(())
}
