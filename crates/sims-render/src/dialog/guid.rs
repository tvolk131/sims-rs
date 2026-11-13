use crate::portrait::{
    cache::Settings,
    images::{Raster, thumbnail},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::object_catalog::Catalog;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Guid {
    guid: u32,
    resource: u32,
    definition: sims_vm::Definition,
    bitmap: u16,
    color: u32,
}

impl Guid {
    pub(super) fn new(
        guid: u32,
        resource: u32,
        catalog: &mut Catalog<'_>,
        color: u32,
    ) -> Result<Self> {
        ensure!(
            catalog
                .portrait_resource(guid)
                .map_err(anyhow::Error::msg)?
                == Some(resource),
            "dialog GUID resource changed"
        );
        let definition = catalog
            .portrait_definition(resource)
            .map_err(anyhow::Error::msg)?;
        let entry = &catalog.definitions.entries[resource as usize - 1];
        ensure!(
            entry.data.len() == 216,
            "dialog GUID definition is not normalized"
        );
        let bitmap = u16::from_le_bytes(entry.data[82..84].try_into().unwrap());
        Ok(Self {
            guid,
            resource,
            definition,
            bitmap,
            color,
        })
    }

    pub(super) fn validate(&self, guid: Option<u32>, resource: Option<u32>) -> Result<()> {
        ensure!(
            guid == Some(self.guid)
                && resource == Some(self.resource)
                && self.definition.guid == self.guid,
            "dialog GUID image owner changed"
        );
        Ok(())
    }

    pub(super) fn render(
        &self,
        catalog: &mut Catalog<'_>,
        index: &sims_content::Index,
        settings: &Settings,
    ) -> Result<Raster> {
        ensure!(
            catalog
                .portrait_resource(self.guid)
                .map_err(anyhow::Error::msg)?
                == Some(self.resource)
                && catalog
                    .portrait_definition(self.resource)
                    .map_err(anyhow::Error::msg)?
                    == self.definition,
            "dialog GUID catalog identity changed"
        );
        let entry = &catalog.definitions.entries[self.resource as usize - 1];
        ensure!(
            entry.data.len() == 216
                && u16::from_le_bytes(entry.data[82..84].try_into().unwrap()) == self.bitmap,
            "dialog GUID catalog bitmap changed"
        );
        ensure!(
            self.color == u32::from(settings.key),
            "dialog GUID display color changed"
        );
        let bytes = catalog
            .portrait_chunk(self.resource, *b"BMP_", self.bitmap)
            .map_err(anyhow::Error::msg)?
            .context("dialog GUID uncached catalog picture generation unsupported")?;
        let mut picture = Raster::from_bmp(&bytes, settings.format)?;
        ensure!(
            [picture.image.width, picture.image.height] == [74, 37],
            "dialog GUID catalog picture regeneration unsupported"
        );
        picture.key = Some(settings.key);
        let mut background = crate::portrait::images::template(index, 4300, settings.format)?
            .context("dialog GUID original catalog thumbnail background absent")?;
        background.key = Some(settings.key);
        let composed = thumbnail::compose(&background, &picture, None)?;
        ensure!(composed.success, "dialog GUID thumbnail composition failed");
        composed
            .image
            .context("dialog GUID thumbnail output absent")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portrait::{images, resize::Format};
    use sims_content::{Index, object_resources::Overlay};
    use std::{collections::BTreeMap, path::Path, sync::Arc};

    #[test]
    #[ignore = "original catalog and BMP assets; headless software image profile"]
    fn original_phone_guid_icon_preserves_saved_catalog_template_and_pixels() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
        let mut catalog = Catalog::household(
            index.clone(),
            "UserData/Neighborhood.iff",
            Overlay::default(),
        )?;
        let resource = catalog
            .portrait_resource(2262875061)
            .map_err(anyhow::Error::msg)?
            .context("original phone GUID absent")?;
        let cache = sims_vm::image_cache::State::default();
        let images = crate::portrait::cache::Images::default();
        let background = images::template(&index, 4300, Format::Rgb565)?
            .context("original action thumbnail background absent")?;
        let settings = Settings {
            layout: [45, 45, 5, 2, 35, 41],
            format: Format::Rgb565,
            key: 0xf81f,
            templates: BTreeMap::new(),
        };
        let icon = Guid::new(2262875061, resource, &mut catalog, u32::from(settings.key))?;
        ensure!(
            icon.definition.source == "GameData_ALL_Recursive/Objects/Objects.far::phones.iff"
                && icon.definition.object == 16807
                && icon.bitmap == 2001
        );
        let before = serde_json::to_value((&cache, &images))?;
        icon.validate(Some(2262875061), Some(resource))?;
        ensure!(icon.validate(Some(7), Some(resource)).is_err());
        ensure!(icon.validate(Some(2262875061), Some(resource + 1)).is_err());
        let direct = icon.render(&mut catalog, &index, &settings)?;
        ensure!(direct.rect() == [0, 0, 45, 45]);
        ensure!(
            direct
                .image
                .pixels
                .iter()
                .enumerate()
                .filter(|(position, pixel)| **pixel
                    != background.image.pixels
                        [(position / 45) * background.image.width + position % 45])
                .count()
                > 20
        );
        let saved = serde_json::to_vec(&icon)?;
        let restored: Guid = serde_json::from_slice(&saved)?;
        let mut restored_catalog = Catalog::household(
            index.clone(),
            "UserData/Neighborhood.iff",
            Overlay::default(),
        )?;
        ensure!(restored.render(&mut restored_catalog, &index, &settings)? == direct);
        let mut wrong = restored.clone();
        wrong.bitmap = 2000;
        ensure!(
            wrong
                .render(&mut restored_catalog, &index, &settings)
                .is_err()
        );
        wrong = restored.clone();
        wrong.definition.guid = 7;
        ensure!(
            wrong
                .render(&mut restored_catalog, &index, &settings)
                .is_err()
        );
        ensure!(serde_json::to_value((&cache, &images))? == before);
        if let Ok(relative) = std::env::var("SIMS_GUID_ICON_OUTPUT") {
            let output = root.join(relative);
            ensure!(output.starts_with(root.join("cache")));
            std::fs::create_dir(&output)?;
            direct.rgba()?.save(output.join("phone-guid.png"))?;
            std::fs::write(output.join("owner.json"), saved)?;
        }
        println!(
            "actual phone GUID2262875061/resource{resource}/OBJD16807/BMP2001: 74x37 source to owned45x45; saved owner and cold catalog pixels equal; non-native RGB565/key profile"
        );
        Ok(())
    }
}
