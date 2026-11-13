//! Asset-backed product data for replacement Buy views. This performs no
//! simulation work, selection, purchase, or acknowledgement of a UI request.
use super::Session;
use anyhow::{Context, Result, ensure};
use sims_content::object_catalog::{Catalog, products::Product};

impl Session {
    pub fn buy_category_labels(
        &self,
    ) -> Result<sims_content::object_catalog::products::labels::Labels> {
        sims_content::object_catalog::products::labels::Labels::load(
            self.index(),
            self.program
                .text_language
                .context("Buy language absent")?
                .try_into()?,
        )
    }
    /// Build once when attaching a product inventory, then retain the result
    /// in that frontend. This is asset loading, not a per-frame property read.
    pub fn buy_products(&self) -> Result<Vec<Product>> {
        let retained = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("Buy catalog has no retained definition owner")?;
        let mut catalog = Catalog::household(
            self.index.clone(),
            &self.house.neighborhood_source,
            Default::default(),
        )?;
        // A changed installation or generated definition must not silently
        // substitute a new registered identity for a saved product selection.
        ensure!(
            catalog.definitions == *retained.catalog()?,
            "Buy catalog differs from retained household definitions"
        );
        let language: u8 = self
            .program
            .text_language
            .context("Buy catalog language absent")?
            .try_into()?;
        let products = catalog.buy_products(language).map_err(anyhow::Error::msg)?;
        for product in &products {
            ensure!(
                retained.history.definitions.get(&product.definition) == Some(&product.source),
                "Buy product source differs from retained household definition"
            );
        }
        Ok(products)
    }
}
