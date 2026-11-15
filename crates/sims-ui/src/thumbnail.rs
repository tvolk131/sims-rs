//! Objects' catalog pictures (thumbnails), as the action queue shows them.
//!
//! **What the original does** (read from its code):
//!
//! - **Which objects have one** (0x4cd830 to 0x4cd8a6): when an object
//!   file is read, each definition gets a catalog item (0x4cbd44, kept at
//!   definition +0x84, 0x457edf), except a tile of a multi-tile object
//!   that is not its lead (0x4575b8) and a person (type 2, 0x4575d2). The
//!   definition needs graphics (0x457588: a master ID or a graphics
//!   count) or an interaction table with entries (0x4575a1, 0x457cb0):
//!   the `TTAB` named by the definition's word at byte 0x0e (0x45726a;
//!   -1 takes the lead's, other negative numbers none), looked up in the
//!   definition's own file and then in its semi-global file (0x4569a3),
//!   with entries when its word +4 is not 0, which is taken here as the
//!   table's count not being 0.
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
//! **Unverified:** that word +4 of the table 0x457cb0 finds is its
//! entries (an empty table's is taken to be 0). No capture shows a queue
//! icon.

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
const TABLE: usize = 0x0e;
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
    /// A type with no file, no definition or no item has none. A file
    /// that cannot be read is an error, and the type is kept with no
    /// item; a thumbnail that does not decode is an error too, but the
    /// item is kept, without its thumbnail, as when it has none. Either
    /// way the type is not read again.
    pub fn read(&mut self, index: &Index, catalog: &Catalog, guid: u32) -> Result<()> {
        if self.items.contains_key(&guid) {
            return Ok(());
        }
        match item(index, catalog, guid) {
            Ok((item, problem)) => {
                self.items.insert(guid, item);
                problem.map_or(Ok(()), Err)
            }
            Err(e) => {
                self.items.insert(guid, None);
                Err(e)
            }
        }
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

/// The catalog item `guid`'s objects show (see the module's notes), and
/// the error its thumbnail gave if it did not decode.
fn item(index: &Index, catalog: &Catalog, guid: u32) -> Result<(Option<Item>, Option<Error>)> {
    let Some(object) = catalog.object(guid) else {
        return Ok((None, None));
    };
    let read = |resource: usize| -> Result<Option<Iff>> {
        let Some(resource) = index.resources.get(resource) else {
            return Ok(None);
        };
        let bytes = resource.read().map_err(Error::Content)?;
        Iff::parse(bytes).map(Some).map_err(Error::Format)
    };
    let Some(iff) = read(object.resource)? else {
        return Ok((None, None));
    };
    let Some(definition) = iff.get(b"OBJD", object.chunk).map(|c| iff.data(c)) else {
        return Ok((None, None));
    };
    let Some(lead) = lead(&iff, definition) else {
        return Ok((None, None));
    };
    if word(lead, KIND) == Some(PERSON) {
        return Ok((None, None));
    }
    let graphics = word(lead, MASTER).unwrap_or(0) != 0 || word(lead, GRAPHICS).unwrap_or(0) != 0;
    if !graphics {
        // The semi-global file is read only when the table is not in the
        // definition's own.
        let semi = match table(lead) {
            Some(id) if iff.get(b"TTAB", id).is_none() => {
                match catalog
                    .semi_globals
                    .get(&object.resource)
                    .copied()
                    .flatten()
                {
                    Some(resource) => read(resource)?,
                    None => None,
                }
            }
            _ => None,
        };
        if !interactions(&iff, semi.as_ref(), lead) {
            return Ok((None, None));
        }
    }
    let (thumbnail, problem) = thumbnail(&iff, lead, graphics);
    Ok((Some(Item { thumbnail }), problem))
}

/// The thumbnail kept for `definition`'s item (0x4cbdc3), and the error
/// its `BMP_` gave if it did not decode, which keeps none.
fn thumbnail(iff: &Iff, definition: &[u8], graphics: bool) -> (Option<Image>, Option<Error>) {
    let id = word(definition, CATALOG_STRINGS).unwrap_or(0) as u16;
    match iff.get(b"BMP_", id).map(|c| bitmap::bmp(iff.data(c))) {
        Some(Ok(image)) => {
            let kept = !graphics || [image.width, image.height] == THUMBNAIL;
            (kept.then_some(image), None)
        }
        Some(Err(e)) => (None, Some(Error::Format(e))),
        None => (None, None),
    }
}

/// The ID of the interaction table `definition` names (0x45726a): its
/// word at byte 0x0e when that is above 0. Here `definition` is a lead,
/// so -1 (the lead's own) names none either.
fn table(definition: &[u8]) -> Option<u16> {
    word(definition, TABLE)
        .filter(|&id| id > 0)
        .map(|id| id as u16)
}

/// Whether `definition`'s interaction table has entries (0x4575a1): the
/// table it names, from its own file or else the semi-global one
/// (0x457cb0, 0x4569a3), whose count (its first word) is not 0.
fn interactions(iff: &Iff, semi: Option<&Iff>, definition: &[u8]) -> bool {
    let Some(id) = table(definition) else {
        return false;
    };
    let chunk = iff
        .get(b"TTAB", id)
        .map(|c| iff.data(c))
        .or_else(|| semi.and_then(|semi| semi.get(b"TTAB", id).map(|c| semi.data(c))));
    chunk
        .and_then(|data| word(data, 0))
        .is_some_and(|count| count != 0)
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

    /// An IFF file of `chunks`, as the game's files are laid out.
    fn iff(chunks: &[(&[u8; 4], u16, Vec<u8>)]) -> Iff {
        let mut bytes = vec![0; 64];
        bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
        for (tag, id, data) in chunks {
            bytes.extend(*tag);
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend(id.to_be_bytes());
            bytes.extend(0u16.to_be_bytes());
            bytes.extend([0; 64]);
            bytes.extend(data);
        }
        Iff::parse(bytes).unwrap()
    }

    /// A definition naming interaction table `table`.
    fn definition(table: i16) -> Vec<u8> {
        let mut bytes = vec![0; 0x60];
        bytes[TABLE..TABLE + 2].copy_from_slice(&table.to_le_bytes());
        bytes
    }

    /// An interaction table's first words: its count, then its version.
    fn table_chunk(count: u16) -> Vec<u8> {
        [count.to_le_bytes(), 8u16.to_le_bytes()].concat()
    }

    #[test]
    fn a_definition_without_graphics_needs_an_interaction_table_with_entries() {
        let own = iff(&[
            (b"TTAB", 300, table_chunk(2)),
            (b"TTAB", 301, table_chunk(0)),
        ]);
        let semi = iff(&[(b"TTAB", 4096, table_chunk(1))]);
        // Its own file's table, with entries or without.
        assert!(interactions(&own, None, &definition(300)));
        assert!(!interactions(&own, Some(&semi), &definition(301)));
        // The semi-global file's, when its own has none of that ID.
        assert!(interactions(&own, Some(&semi), &definition(4096)));
        assert!(!interactions(&own, None, &definition(4096)));
        // No table: 0, -1 on a lead, or another negative number.
        for id in [0, -1, -2] {
            assert!(!interactions(&own, Some(&semi), &definition(id)));
        }
    }

    #[test]
    fn a_thumbnail_that_does_not_decode_is_an_error_and_keeps_none() {
        let mut definition = definition(0);
        definition[CATALOG_STRINGS..CATALOG_STRINGS + 2].copy_from_slice(&5u16.to_le_bytes());
        let file = iff(&[(b"BMP_", 5, b"XX not a bitmap".to_vec())]);
        let (kept, problem) = thumbnail(&file, &definition, true);
        assert!(kept.is_none());
        assert!(problem.is_some());
        // No BMP_ of that ID is no thumbnail and no error.
        let (kept, problem) = thumbnail(&iff(&[]), &definition, true);
        assert!(kept.is_none() && problem.is_none());
    }
}
