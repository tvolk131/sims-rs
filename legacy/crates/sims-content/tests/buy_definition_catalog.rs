use anyhow::{Result, ensure};
use sims_content::{Index, object_catalog::Catalog, object_resources::Overlay};
use sims_sim::object_catalog::move_tool::{Fields, Groups, selected::Masters};
use std::sync::Arc;

#[test]
#[ignore = "private original Complete Collection catalogs; read-only master/selection validation"]
fn original_catalog_master_selection_and_cache_restore_preserve_resource_identity() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_BUY_DEFINITION_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut reports = vec![];
    for neighborhood in ["UserData/Neighborhood.iff", "UserData2/Neighborhood.iff"] {
        let catalog = Catalog::household(index.clone(), neighborhood, Overlay::default())?;
        let identity = catalog.identity().map_err(anyhow::Error::msg)?;
        let mut groups = Groups::default();
        catalog
            .populate_move_tool_groups(&mut groups)
            .map_err(anyhow::Error::msg)?;
        let mut masters = Masters::default();
        let mut missing = vec![];
        let mut multipart = 0;
        let mut selected = 0;
        let mut special = 0;
        let word =
            |data: &[u8], offset| i16::from_le_bytes(data[offset..offset + 2].try_into().unwrap());
        for (row, entry) in catalog.definitions.entries.iter().enumerate() {
            let id = row as u32 + 1;
            let group = word(&entry.data, 0x14);
            let master = masters
                .resolve(&catalog.definitions, id)
                .map_err(anyhow::Error::msg)?;
            let Some(master) = master else {
                ensure!(group != 0 && !masters.0.contains_key(&id));
                missing
                    .push(serde_json::json!({"id":id,"name":String::from_utf8_lossy(&entry.name)}));
                continue;
            };
            let target = &catalog.definitions.entries[master as usize - 1];
            if group != 0 {
                multipart += 1;
                ensure!(
                    target.key.resource == entry.key.resource
                        && word(&target.data, 0x14) == group
                        && word(&target.data, 0x16) == -1
                );
            } else {
                ensure!(master == id);
            }
            let mut f = Fields::constructor_fields();
            f.argument = row as i32;
            f.position = [3, 7];
            let answer = groups
                .select_definition(&mut f, &mut masters, &catalog.definitions, id)
                .map_err(anyhow::Error::msg)?;
            let mut cold: Masters = serde_json::from_slice(&serde_json::to_vec(&masters)?)?;
            let mut restored = Fields::constructor_fields();
            restored.argument = row as i32;
            restored.position = [3, 7];
            let cold_answer = groups
                .select_definition(&mut restored, &mut cold, &catalog.definitions, id)
                .map_err(anyhow::Error::msg)?;
            ensure!(f == restored && answer == cold_answer && masters == cold);
            ensure!(
                f.argument == row as i32 && f.position == [3, 7] && f.definition == Some(master)
            );
            selected += 1;
            special += usize::from(answer != 0);
        }
        ensure!(selected > 0 && multipart > 0 && special > 0);
        ensure!(identity == catalog.identity().map_err(anyhow::Error::msg)?);
        reports.push(serde_json::json!({"neighborhood":neighborhood,"catalog_identity":identity,
            "definitions":catalog.definitions.entries.len(),"selections":selected,"cold_restores":selected,
            "multipart_definitions":multipart,"special_selections":special,"missing_masters":missing,"catalog_unchanged":true}));
    }
    std::fs::write(
        out,
        serde_json::to_vec_pretty(
            &serde_json::json!({"reports":reports,"native_execution":false,
        "scope":"Original installed definition/master classification, not object purchase or full Buy-mode binding"}),
        )?,
    )?;
    Ok(())
}
