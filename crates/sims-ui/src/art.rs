//! The original artwork the interface draws with, loaded once.

use crate::font::Font;
use crate::{Error, Result};
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use std::collections::BTreeMap;

/// The color the interface's bitmaps use for "no pixel here". The old
/// prototype keyed the same color (legacy/crates/sims-render/src/dialog).
/// The TGAs have an alpha channel instead and no pixel of this color.
pub const KEY: [u8; 3] = [255, 0, 255];

/// The font sizes the interface uses (`Fonts/variablesans_NN.ffn`).
pub const FONT_SIZES: [u8; 6] = [8, 9, 10, 11, 12, 14];

/// Bitmaps by resource ID, and fonts by size.
#[derive(Debug, Clone, Default)]
pub struct Art {
    images: BTreeMap<i32, Image>,
    fonts: BTreeMap<u8, Font>,
    /// Requested resources the install's tables do not name.
    pub missing: Vec<i32>,
}

impl Art {
    /// Loads resources `ids` in `language` (falling back to the rows for
    /// every language) and the fonts of [`FONT_SIZES`].
    pub fn load(ui: &UiGraphics, language: u8, ids: &[i32]) -> Result<Self> {
        let mut art = Self::default();
        for &id in ids {
            match ui.bitmap(id, language).map_err(Error::Content)? {
                Some(mut image) => {
                    image.key(KEY);
                    art.images.insert(id, image);
                }
                None => art.missing.push(id),
            }
        }
        for size in FONT_SIZES {
            let bytes = ui
                .read(&format!("Fonts/variablesans_{size:02}.ffn"))
                .map_err(Error::Content)?;
            let font = sims_formats::ffn::Font::parse(&bytes).map_err(Error::Format)?;
            art.fonts.insert(size, Font::new(font));
        }
        Ok(art)
    }

    /// Adds or replaces an image, as tests and hosts with their own art do.
    pub fn insert(&mut self, id: i32, image: Image) {
        self.images.insert(id, image);
    }

    /// Adds or replaces a font.
    pub fn insert_font(&mut self, size: u8, font: Font) {
        self.fonts.insert(size, font);
    }

    pub fn image(&self, id: i32) -> Option<&Image> {
        self.images.get(&id)
    }

    pub fn font(&self, size: u8) -> Option<&Font> {
        self.fonts.get(&size)
    }

    /// Cell `cell` of a button strip: the original's buttons keep their
    /// states side by side in one bitmap, `cells` of them.
    pub fn cell(&self, id: i32, cells: u32, cell: u32) -> Option<Image> {
        let image = self.image(id)?;
        let width = image.width / cells.max(1);
        image.crop(width.checked_mul(cell)?, 0, width, image.height)
    }
}
