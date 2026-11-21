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
pub const FONT_SIZES: [u8; 7] = [7, 8, 9, 10, 11, 12, 14];

/// Bitmaps by resource ID, and fonts by size.
#[derive(Debug, Clone, Default)]
pub struct Art {
    images: BTreeMap<i32, Image>,
    fonts: BTreeMap<u8, Font>,
    /// Requested resources the install's tables do not name.
    pub missing: Vec<i32>,
}

/// Un-premultiplies a UI TGA's partially transparent pixels (`0 < a < 255`)
/// as the original's TGA loader does at `0x4deb3d..0x4deb9d` when
/// `[0x6c8c3c] == 1` (true for every UI TGA except the Magicland mist
/// overlays `0x1655..0x1657`): `c = trunc(c * (255.0f / a) + 0.5f) as u8`,
/// in 24-bit single precision (`0x4fad5c`) taking the low 8 bits on store
/// (`0x5dff82..0x5dff94`). Opaque and colorkeyed BMP pixels (`a == 0` or
/// `a == 255`) are untouched.
fn unpremultiply_tga(image: &mut Image) {
    for px in image.rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a > 0 && a < 255 {
            let scale = 255.0f32 / f32::from(a);
            for ch in &mut px[..3] {
                *ch = ((f32::from(*ch) * scale + 0.5f32).trunc() as i32) as u8;
            }
        }
    }
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
                    unpremultiply_tga(&mut image);
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

    /// Loads resource `id` in `language` too, if it can: one the tables
    /// do not name, or whose file is missing or unreadable, is only
    /// noted in [`Art::missing`], and what draws it goes without.
    pub fn load_optional(&mut self, ui: &UiGraphics, language: u8, id: i32) {
        match ui.bitmap(id, language) {
            Ok(Some(mut image)) => {
                image.key(KEY);
                unpremultiply_tga(&mut image);
                self.images.insert(id, image);
            }
            Ok(None) | Err(_) => self.missing.push(id),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpremultiply_tga_restores_straight_rgb_for_partial_alpha() {
        let mut image = Image {
            width: 4,
            height: 1,
            rgba: vec![
                // Transparent (alpha 0): untouched.
                10, 20, 30, 0, // Opaque (alpha 255): untouched.
                10, 20, 30, 255,
                // Single-precision rounding (0x4fad5c): shipped UniversalBack.TGA
                // (160, 75) has blue 56, alpha 112 -> 127 (not f64's 128).
                64, 32, 56, 112,
                // Low-byte wrap (0x5dff82..0x5dff94): shipped HotspotPopupTiler.TGA
                // pixel has blue 82, alpha 35 -> trunc(597.928) & 0xff = 85.
                0, 35, 82, 35,
            ],
        };
        unpremultiply_tga(&mut image);
        assert_eq!(
            image.rgba,
            vec![
                10, 20, 30, 0, 10, 20, 30, 255, 146, 73, 127, 112, 0, 255, 85, 35,
            ]
        );
    }
}
