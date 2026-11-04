//! Product membership and metadata from all installed Complete Collection packs.
use anyhow::{Result, ensure};
use sims_content::{Index, object_catalog::Catalog, object_resources::Overlay};
use sims_sim::object_catalog::products::{Eligibility, Masks};
use std::sync::Arc;

#[test]
#[ignore = "private Complete Collection assets; read-only catalog"]
fn original_buy_products_preserve_definitions_and_seven_catalog_filters() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_BUY_PRODUCTS_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut reports = vec![];
    for neighborhood in ["UserData/Neighborhood.iff", "UserData2/Neighborhood.iff"] {
        let mut catalog = Catalog::household(index.clone(), neighborhood, Overlay::default())?;
        let before = catalog.identity().map_err(anyhow::Error::msg)?;
        let products = catalog.buy_products(1).map_err(anyhow::Error::msg)?;
        let cold: Vec<sims_content::object_catalog::products::Product> =
            serde_json::from_slice(&serde_json::to_vec(&products)?)?;
        ensure!(cold == products && products.len() > 100);
        let mut counts = vec![];
        for c in 0..7 {
            let mut matrix = [[0usize; 8]; 8];
            for (row, columns) in matrix.iter_mut().enumerate() {
                for (col, count) in columns.iter_mut().enumerate() {
                    let mut masks = Masks::default();
                    masks.update(c, &[[row as i32, col as i32]; 7]);
                    *count = products.iter().filter(|p| p.visible(c, masks)).count();
                }
            }
            counts.push(matrix);
        }
        let mut fallback = 0;
        let mut ttab = 0;
        for p in &products {
            ensure!(
                p.source
                    == catalog
                        .portrait_definition(p.definition)
                        .map_err(anyhow::Error::msg)?
            );
            ensure!(p.fields.kind != 2 && (p.fields.group == 0 || p.fields.subindex == -1));
            ensure!(!p.name.is_empty());
            fallback += usize::from(p.name.starts_with(b"*"));
            ttab += usize::from(p.fields.eligibility() == Eligibility::Interactions);
        }
        ensure!(
            products
                .windows(2)
                .all(|p| (p[0].fields.price, &p[0].name) <= (p[1].fields.price, &p[1].name))
        );
        ensure!(before == catalog.identity().map_err(anyhow::Error::msg)?);
        reports.push(
            serde_json::json!({"neighborhood":neighborhood,"identity":before,
            "definitions":catalog.definitions.entries.len(),"products":products.len(),
            "interaction_table_members":ttab,"fallback_names":fallback,"category_counts":counts,
            "definitions_unchanged":true,"serde_equal":true,"items":products}),
        );
    }
    std::fs::write(
        out,
        serde_json::to_vec_pretty(&serde_json::json!({"reports":reports,
        "original_execution":false,"scope":"Read-only asset-backed product membership and metadata; purchase and UI integration separate"}))?,
    )?;
    Ok(())
}
