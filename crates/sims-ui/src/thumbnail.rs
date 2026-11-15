//! Objects' catalog pictures (thumbnails), as the action queue shows them.
//!
//! **What the original does** (read from its code):
//!
//! - **Which objects have one** (0x4cd830 to 0x4cd8a6): when an object
//!   file is read, each definition gets a catalog item (0x4cbd44, kept at
//!   definition +0x84, 0x457edf), except a tile of a multi-tile object
//!   that is not its lead (0x4575b8) and a person (type 2, 0x4575d2). The
//!   definition needs graphics (0x457588: a master ID or a graphics
//!   count) or what 0x4575a1 finds in its file.
//! - **Whose item an object shows** (0x45728d, 0x43ae0a): its own, or for
//!   a tile of a multi-tile object, the item of the first definition in
//!   the same file with the same master ID and sub-index -1 (0x43adf7).
//!   None found, no item.
//! - **The thumbnail** (0x4cbdc3): the item's file's `BMP_` whose ID is
//!   the definition's catalog strings ID (word at byte 0x52). For a
//!   definition with graphics, it is kept only if it is 74 by 37: two 37
//!   by 37 cells, the object on the catalog's normal background and on
//!   its highlighted one. Global 0x6f2a3c (0 in our capture) would drop
//!   it. With no thumbnail kept, the original renders the object into one
//!   (0x4e57a6), which is **not ported**.
//! - **The icon** (0x4ccaa0): a 45 by 45 picture, the first cell of
//!   `kCatalogTemplate` (`CPanel/Catalog/ThumbTemplate.bmp`, four 45 by 45
//!   cells, loaded by 0x4ccfb0), then the thumbnail over it, its left
//!   half when it is wider than 45, centred (rounding toward zero). An
//!   item with no thumbnail leaves the template alone, and the queue
//!   shows that.
//!
//! **Unverified:** what 0x4575a1 finds for a definition without graphics
//! (0x457cb0, a cast of its file); here such a definition has an item. No
//! capture shows a queue icon.

use crate::art::Art;
use crate::{Error, Result};
use sims_content::{Catalog, Index};
use sims_formats::bitmap::{self, Image};
use sims_formats::iff::Iff;
use std::collections::BTreeMap;

/// `kCatalogTemplate` (`UIGraphics/Res_CPanel.h`).
pub const TEMPLATE: i32 = 4300;

/// Every resource a thumbnail's icon draws, for [`Art::load`].
pub const RESOURCES: [i32; 1] = [TEMPLATE];

/// The size the icon is made at, and the template's cell size.
pub const SIZE: u32 = 45;

/// The size of a kept thumbnail of a definition with graphics.
pub const THUMBNAIL: [u32; 2] = [74, 37];

/// Definition words, by byte offset into an `OBJD`.
const GRAPHICS: usize = 0x08;
const KIND: usize = 0x12;
const MASTER: usize = 0x14;
const SUB_INDEX: usize = 0x16;
const CATALOG_STRINGS: usize = 0x52;
/// A person's type.
const PERSON: i16 = 2;

/// An object type's catalog item, as far as the queue uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The thumbnail kept, if any.
    pub thumbnail: Option<Image>,
}

/// The catalog items of object types, read once each, by GUID.
#[derive(Debug, Clone, Default)]
pub struct Thumbnails {
    items: BTreeMap<u32, Option<Item>>,
}

impl Thumbnails {
    /// Reads the catalog item `guid`'s objects show, if it was not read.
    /// A type with no file, no definition or no item has none; a
    /// thumbnail that does not decode is an error.
    pub fn read(&mut self, index: &Index, catalog: &Catalog, guid: u32) -> Result<()> {
        if self.items.contains_key(&guid) {
            return Ok(());
        }
        let item = item(index, catalog, guid)?;
        self.items.insert(guid, item);
        Ok(())
    }

    /// Sets `guid`'s item, as tests and hosts with their own art do.
    pub fn insert(&mut self, guid: u32, item: Option<Item>) {
        self.items.insert(guid, item);
    }

    /// `guid`'s item, if it was read and there is one.
    pub fn item(&self, guid: u32) -> Option<&Item> {
        self.items.get(&guid)?.as_ref()
    }

    /// The 45 by 45 picture of `guid`'s objects (0x4ccaa0), or None when
    /// they have no catalog item (or it was not read), or the template is
    /// not loaded.
    pub fn icon(&self, art: &Art, guid: u32) -> Option<Image> {
        let item = self.item(guid)?;
        let mut icon = art.cell(TEMPLATE, 4, 0)?;
        if let Some(thumbnail) = &item.thumbnail {
            paste(&mut icon, thumbnail);
        }
        Some(icon)
    }
}

/// Puts `thumbnail` over `icon` as 0x4ccaa0 blits it: its left half when
/// it is wider than the icon, centred, offsets rounding toward zero.
fn paste(icon: &mut Image, thumbnail: &Image) {
    let size = i64::from(SIZE);
    let width = i64::from(thumbnail.width);
    let width = if width > size { width / 2 } else { width };
    let height = i64::from(thumbnail.height);
    let left = (size - width) / 2;
    let top = (size - height) / 2;
    for y in 0..height {
        for x in 0..width {
            let (tx, ty) = (left + x, top + y);
            if !(0..i64::from(icon.width)).contains(&tx)
                || !(0..i64::from(icon.height)).contains(&ty)
            {
                continue;
            }
            let (Ok(sx), Ok(sy), Ok(tx), Ok(ty)) = (
                u32::try_from(x),
                u32::try_from(y),
                usize::try_from(tx),
                usize::try_from(ty),
            ) else {
                continue;
            };
            let Some(pixel) = thumbnail.pixel(sx, sy) else {
                continue;
            };
            let at = (ty * icon.width as usize + tx) * 4;
            if let Some(target) = icon.rgba.get_mut(at..at + 4) {
                target.copy_from_slice(&pixel);
            }
        }
    }
}

/// The catalog item `guid`'s objects show (see the module's notes).
fn item(index: &Index, catalog: &Catalog, guid: u32) -> Result<Option<Item>> {
    let Some(object) = catalog.object(guid) else {
        return Ok(None);
    };
    let Some(resource) = index.resources.get(object.resource) else {
        return Ok(None);
    };
    let iff = Iff::parse(resource.read().map_err(Error::Content)?).map_err(Error::Format)?;
    let Some(definition) = iff.get(b"OBJD", object.chunk).map(|c| iff.data(c)) else {
        return Ok(None);
    };
    let Some(lead) = lead(&iff, definition) else {
        return Ok(None);
    };
    if word(lead, KIND) == Some(PERSON) {
        return Ok(None);
    }
    let graphics = word(lead, MASTER).unwrap_or(0) != 0 || word(lead, GRAPHICS).unwrap_or(0) != 0;
    let id = word(lead, CATALOG_STRINGS).unwrap_or(0) as u16;
    let thumbnail = match iff.get(b"BMP_", id) {
        Some(chunk) => Some(bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?),
        None => None,
    }
    .filter(|image| !graphics || [image.width, image.height] == THUMBNAIL);
    Ok(Some(Item { thumbnail }))
}

/// The definition whose item `definition`'s objects show: itself, or for
/// a tile of a multi-tile object, its file's lead tile (0x43ae0a).
fn lead<'a>(iff: &'a Iff, definition: &'a [u8]) -> Option<&'a [u8]> {
    let master = word(definition, MASTER)?;
    if master == 0 {
        return Some(definition);
    }
    iff.all(b"OBJD")
        .map(|chunk| iff.data(chunk))
        .find(|d| word(d, MASTER) == Some(master) && word(d, SUB_INDEX) == Some(-1))
}

fn word(definition: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes(
        definition.get(at..at + 2)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, at: impl Fn(u32, u32) -> [u8; 4]) -> Image {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend(at(x, y));
            }
        }
        Image {
            width,
            height,
            rgba,
        }
    }

    fn art() -> Art {
        let mut art = Art::default();
        // Four cells: the first blue, the others grey.
        art.insert(
            TEMPLATE,
            image(180, 45, |x, _| {
                if x < 45 {
                    [0, 0, 90, 255]
                } else {
                    [50, 50, 50, 255]
                }
            }),
        );
        art
    }

    #[test]
    fn a_thumbnails_left_cell_is_centred_on_the_templates_first_cell() {
        let mut thumbnails = Thumbnails::default();
        // The left cell red, the right green.
        let thumbnail = image(74, 37, |x, _| {
            if x < 37 {
                [200, 0, 0, 255]
            } else {
                [0, 200, 0, 255]
            }
        });
        thumbnails.insert(
            7,
            Some(Item {
                thumbnail: Some(thumbnail),
            }),
        );
        let icon = thumbnails.icon(&art(), 7).unwrap();
        assert_eq!([icon.width, icon.height], [45, 45]);
        // (45 - 37) / 2 = 4 on each side.
        assert_eq!(icon.pixel(3, 20), Some([0, 0, 90, 255]));
        assert_eq!(icon.pixel(4, 4), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(40, 40), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(41, 20), Some([0, 0, 90, 255]));
        assert_eq!(icon.pixel(20, 41), Some([0, 0, 90, 255]));
    }

    #[test]
    fn a_narrow_thumbnail_is_centred_whole_rounding_toward_zero() {
        let mut thumbnails = Thumbnails::default();
        thumbnails.insert(
            7,
            Some(Item {
                thumbnail: Some(image(16, 16, |_, _| [200, 0, 0, 255])),
            }),
        );
        let icon = thumbnails.icon(&art(), 7).unwrap();
        // (45 - 16) / 2 = 14.
        assert_eq!(icon.pixel(13, 14), Some([0, 0, 90, 255]));
        assert_eq!(icon.pixel(14, 14), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(29, 29), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(30, 29), Some([0, 0, 90, 255]));
    }

    #[test]
    fn an_item_without_a_thumbnail_shows_the_template_and_no_item_shows_nothing() {
        let mut thumbnails = Thumbnails::default();
        thumbnails.insert(7, Some(Item { thumbnail: None }));
        thumbnails.insert(8, None);
        let icon = thumbnails.icon(&art(), 7).unwrap();
        assert!(icon.rgba.chunks(4).all(|p| p == [0, 0, 90, 255]));
        assert!(thumbnails.icon(&art(), 8).is_none());
        assert!(thumbnails.icon(&art(), 9).is_none());
    }
}
