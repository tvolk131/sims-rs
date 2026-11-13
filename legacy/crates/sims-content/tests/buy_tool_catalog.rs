use anyhow::{Result, ensure};
use sims_content::{Index, object_catalog::Catalog, object_resources::Overlay};
use sims_sim::object_catalog::move_tool::{Activation, Fields, Groups};
use std::sync::Arc;

#[test]
#[ignore = "requires original Complete Collection files; read-only catalog classification"]
fn original_household_catalogs_supply_the_move_tools_seven_ranges() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = repo.join(std::env::var("SIMS_BUY_TOOL_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")) && !output.exists());
    let index = Arc::new(Index::scan(&repo.join("game-data/cabinet-extracted"))?);
    let mut reports = vec![];
    // Newbie and Goth share UserData's registered definitions. Mashuga uses
    // UserData2's distinct person-resource order and GUID bindings.
    for neighborhood in ["UserData/Neighborhood.iff", "UserData2/Neighborhood.iff"] {
        let catalog = Catalog::household(index.clone(), neighborhood, Overlay::default())?;
        let before = catalog.identity().map_err(anyhow::Error::msg)?;
        let mut groups = Groups::default();
        catalog
            .populate_move_tool_groups(&mut groups)
            .map_err(anyhow::Error::msg)?;
        let mut activated = 0;
        let mut examples = vec![];
        for (group, list) in groups.0.iter().enumerate() {
            ensure!(
                !list.guids.is_empty(),
                "original group{} is empty",
                group + 1
            );
            for (n, guid) in list.guids.iter().enumerate() {
                let argument = list.first.wrapping_add(n as i32);
                ensure!(argument == i32::from(argument as i16));
                let id = groups
                    .lookup(group, argument, &catalog.definitions)
                    .map_err(anyhow::Error::msg)?
                    .ok_or_else(|| anyhow::anyhow!("original GUID{guid:08x} absent"))?;
                let mut fields = Fields::constructor_fields();
                let mut call = Activation::new(argument as u32);
                ensure!(
                    call.advance(&mut fields, &groups, &catalog.definitions, 0)
                        .map_err(anyhow::Error::msg)?
                );
                let saved = serde_json::to_vec(&(&call, &fields, &groups))?;
                let (mut cold, mut restored, cold_groups): (Activation, Fields, Groups) =
                    serde_json::from_slice(&saved)?;
                for (c, f, g) in [
                    (&mut call, &mut fields, &groups),
                    (&mut cold, &mut restored, &cold_groups),
                ] {
                    // Explicit child boundary. This test validates catalog
                    // resolution/activation, not object teardown or placement.
                    f.argument = -1;
                    c.acknowledge_deactivation().map_err(anyhow::Error::msg)?;
                    ensure!(
                        !c.advance(f, g, &catalog.definitions, 0)
                            .map_err(anyhow::Error::msg)?
                    );
                    ensure!(c.complete());
                }
                ensure!(fields == restored && call == cold);
                ensure!(fields.definition == Some(id) && fields.mode == group as i32 + 3);
                if n == 0 {
                    let e = &catalog.definitions.entries[id as usize - 1];
                    examples.push(serde_json::json!({"group":group+1,"argument":argument,"guid":format!("{guid:08x}"),
                        "definition":id,"name":String::from_utf8_lossy(&e.name),"fields":fields}));
                }
                activated += 1;
            }
        }
        let first = groups.clone();
        catalog
            .populate_move_tool_groups(&mut groups)
            .map_err(anyhow::Error::msg)?;
        ensure!(first == groups);
        ensure!(before == catalog.identity().map_err(anyhow::Error::msg)?);
        reports.push(serde_json::json!({"neighborhood":neighborhood,"catalog_identity":before,
            "definitions":catalog.definitions.entries.len(),"group_counts":groups.0.iter().map(|g|g.guids.len()).collect::<Vec<_>>(),
            "activations":activated,"cold_restores":activated,"catalog_unchanged":true,"examples":examples}));
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({"reports":reports,
        "native_execution":false,"scope":"Installed original OBJD definitions through source-derived move classification/activation. Virtual deactivation remains a supplied child; no VM, purchase or placement mutation."}))?,
    )?;
    Ok(())
}
