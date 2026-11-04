//! Resource selection used by 43878f, 45764f and 4c8e0d. This adapter reads
//! the exact registered definition/resource and preserves original ordering.
//! It does not draw, run native conversion callbacks, or replace missing frames.
use crate::object_catalog::Catalog;
use anyhow::{Context, Result, ensure};
use sims_formats::sprite::{DrawGroup, DrawView, SpriteSet};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Definition {
    pub resource: u32,
    pub sprite_wrapper: u32,
    pub first_graphic: i16,
    pub graphic_count: i16,
    pub group: i16,
    pub subindex: i16,
    pub direction: i16,
    pub preview_index: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Encoding {
    Legacy,
    Spr2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Frame {
    pub encoding: Encoding,
    pub version: u32,
    /// Native top, left, bottom, right. Raw SPR2's bounds helper uses zero
    /// origin and signed low-word dimensions, independently of its pixel data.
    pub rect: [i16; 4],
}

#[derive(Debug)]
struct Sprites {
    encoding: Encoding,
    version: u32,
    frames: Vec<[i16; 4]>,
}

/// 45768a's signed-word interval, including the truncated end expression.
/// Reject the native infinite-wrap case explicitly instead of hanging a host.
pub fn graphic_ids(first: i16, count: i16) -> Result<Vec<i16>> {
    let last = first.wrapping_add(count).wrapping_sub(1);
    if first > last {
        return Ok(Vec::new());
    }
    ensure!(
        last != i16::MAX,
        "native drawing-group interval would wrap indefinitely"
    );
    Ok((first..=last).collect())
}

/// 407847: first matching direction-mask AND exact zoom, in file order.
pub fn view(group: &DrawGroup, facing: i32, zoom: i32) -> Option<&DrawView> {
    let bit = 1_u32 << (facing as u8 & 31);
    group
        .views
        .iter()
        .find(|v| v.direction & bit != 0 && v.zoom as i32 == zoom)
}

/// A cache is tied to one borrowed catalog, so identical numeric handles from
/// a separately restored catalog can never silently reuse this cache's assets.
pub struct Sources<'a, 'files> {
    catalog: &'a mut Catalog<'files>,
    groups: BTreeMap<u32, (Definition, Vec<Option<DrawGroup>>)>,
    sprites: BTreeMap<(u32, u32), Option<Sprites>>,
}

impl<'a, 'files> Sources<'a, 'files> {
    pub fn new(catalog: &'a mut Catalog<'files>) -> Self {
        Self {
            catalog,
            groups: BTreeMap::new(),
            sprites: BTreeMap::new(),
        }
    }

    pub fn definition(&self, handle: u32) -> Result<Definition> {
        let resource = self
            .catalog
            .portrait_file(handle)
            .map_err(anyhow::Error::msg)?;
        let e = &self.catalog.definitions.entries[handle as usize - 1];
        ensure!(e.data.len() == 216, "drawing definition is not normalized");
        let word = |n: usize| i16::from_le_bytes(e.data[n * 2..n * 2 + 2].try_into().unwrap());
        Ok(Definition {
            resource,
            sprite_wrapper: e.sprite_resource,
            first_graphic: word(3),
            graphic_count: word(4),
            group: word(10),
            subindex: word(11),
            direction: word(61),
            preview_index: word(75),
        })
    }

    /// 43ad79: same retained object-resource instance and group, excluding
    /// catalog masters, in registered-definition order. Preview projection and
    /// its separate sort must happen after this enumeration.
    pub fn parts(&self, handle: u32) -> Result<Vec<(u32, i16)>> {
        let master = self.definition(handle)?;
        if master.group == 0 {
            return Ok(Vec::new());
        }
        ensure!(
            self.catalog.definitions.entries.len() <= i16::MAX as usize,
            "native multipart registry index would truncate"
        );
        let mut parts = Vec::new();
        for (index, entry) in self.catalog.definitions.entries.iter().enumerate() {
            if entry.key.resource != master.resource {
                continue;
            }
            ensure!(
                entry.data.len() == 216,
                "multipart drawing definition is not normalized"
            );
            let group = i16::from_le_bytes(entry.data[20..22].try_into().unwrap());
            let subindex = i16::from_le_bytes(entry.data[22..24].try_into().unwrap());
            if group == master.group && subindex != -1 {
                parts.push((index as u32 + 1, subindex));
            }
        }
        Ok(parts)
    }

    fn chunk(&mut self, resource: u32, tag: [u8; 4], id: u32) -> Result<Option<Vec<u8>>> {
        let h = self
            .catalog
            .resources
            .by_id(resource, u32::from_be_bytes(tag), id, 0)
            .map_err(anyhow::Error::msg)?;
        if h == 0 {
            return Ok(None);
        }
        let bytes = self.catalog.resources.data(h).map(|b| b.to_vec());
        self.catalog
            .resources
            .release_chunk(resource, h)
            .map_err(anyhow::Error::msg)?;
        Ok(Some(bytes.map_err(anyhow::Error::msg)?))
    }

    /// The caller's selector is an index, not a DGRP chunk ID. 438954 resets
    /// selectors >= signed graphic_count to zero; negative selectors remain
    /// negative and the vector getter returns no group.
    pub fn selected_view(
        &mut self,
        handle: u32,
        selector: i32,
        facing: i32,
        zoom: i32,
    ) -> Result<Option<DrawView>> {
        let definition = self.definition(handle)?;
        if let Some((cached, _)) = self.groups.get(&handle) {
            ensure!(
                *cached == definition,
                "registered drawing definition changed while cached"
            );
        } else {
            // The native first lookup initializes the whole vector in ID order,
            // including empty group objects for missing chunks.
            let mut groups = Vec::new();
            for id in graphic_ids(definition.first_graphic, definition.graphic_count)? {
                let bytes = self.chunk(definition.resource, *b"DGRP", id as i32 as u32)?;
                groups.push(bytes.map(|b| DrawGroup::parse(&b)).transpose()?);
            }
            self.groups.insert(handle, (definition, groups));
        }
        let index = if selector >= i32::from(definition.graphic_count) {
            0
        } else {
            selector
        };
        Ok(usize::try_from(index)
            .ok()
            .and_then(|n| self.groups[&handle].1.get(n))
            .and_then(Option::as_ref)
            .and_then(|g| view(g, facing, zoom))
            .cloned())
    }

    fn load_sprites(&mut self, handle: u32, id: u32) -> Result<Option<&Sprites>> {
        let definition = self.definition(handle)?;
        let key = (definition.sprite_wrapper, id);
        if !self.sprites.contains_key(&key) {
            let wrapper = self
                .catalog
                .sprites
                .entries
                .iter()
                .find(|e| e.wrapper == definition.sprite_wrapper && e.references != 0)
                .context("object sprite-resource wrapper absent")?;
            let resource = wrapper.resource;
            // 4c8e0d gives SPR# priority. An unsupported/malformed existing
            // SPR# must not silently fall back to SPR2 with the same ID.
            let sprites = if let Some(bytes) = self.chunk(resource, *b"SPR#", id)? {
                Some(legacy_metadata(&bytes)?)
            } else if let Some(bytes) = self.chunk(resource, *b"SPR2", id)? {
                Some(spr2_metadata(&bytes)?)
            } else {
                None
            };
            self.sprites.insert(key, sprites);
        }
        Ok(self.sprites[&key].as_ref())
    }

    /// 4c8deb: absent resources yield zero. Versions outside the adapter's
    /// supported metadata formats remain an explicit host error.
    pub fn frame_count(&mut self, handle: u32, id: u32) -> Result<i32> {
        Ok(self
            .load_sprites(handle, id)?
            .map_or(0, |s| s.frames.len() as i32))
    }

    /// Decode the selected raw SPR2 through the existing mounted-asset codec.
    /// This is a software presentation endpoint, not the native pixel blitter.
    /// Resource priority is shared with the metadata getters; an existing SPR#
    /// is never bypassed in favor of SPR2.
    pub fn decode_spr2(
        &mut self,
        handle: u32,
        id: u32,
        frame: i32,
    ) -> Result<sims_formats::sprite::Frame> {
        let metadata = self.frame(handle, id, frame)?.context("sprite absent")?;
        ensure!(
            metadata.encoding == Encoding::Spr2,
            "software object preview requires SPR2"
        );
        let definition = self.definition(handle)?;
        let resource = self
            .catalog
            .sprites
            .entries
            .iter()
            .find(|e| e.wrapper == definition.sprite_wrapper && e.references != 0)
            .context("object sprite-resource wrapper absent")?
            .resource;
        let bytes = self
            .chunk(resource, *b"SPR2", id)?
            .context("SPR2 disappeared")?;
        let set = SpriteSet::parse(&bytes)?;
        ensure!(
            set.version == 1000,
            "software preview SPR2 version unsupported"
        );
        let palette = self
            .chunk(resource, *b"PALT", set.default_palette)?
            .context("object preview palette absent")?;
        let palette = sims_formats::sprite::Palette::parse(&palette)?;
        let range = set
            .frames
            .get(usize::try_from(frame)?)
            .context("SPR2 frame absent")?;
        Ok(sims_formats::sprite::Frame::decode(
            &bytes[range.clone()],
            &palette,
        )?)
    }

    /// 4c8d5c metadata only. The original caller separately checks count and
    /// emptiness; unavailable host memory remains an error instead of an image.
    pub fn frame(&mut self, handle: u32, id: u32, index: i32) -> Result<Option<Frame>> {
        let Some(s) = self.load_sprites(handle, id)? else {
            return Ok(None);
        };
        let rect = *s
            .frames
            .get(usize::try_from(index).context("negative native sprite-frame index")?)
            .context("native sprite-frame index outside resource")?;
        Ok(Some(Frame {
            encoding: s.encoding,
            version: s.version,
            rect,
        }))
    }
}

fn spr2_metadata(bytes: &[u8]) -> Result<Sprites> {
    let set = SpriteSet::parse(bytes)?;
    ensure!(
        set.version == 1000,
        "native raw SPR2 metadata version unavailable"
    );
    let mut frames = Vec::with_capacity(set.frames.len());
    for r in set.frames {
        let b = &bytes[r];
        let width = i16::from_le_bytes(b[0..2].try_into().unwrap());
        let height = i16::from_le_bytes(b[2..4].try_into().unwrap());
        frames.push([0, 0, height, width]);
    }
    Ok(Sprites {
        encoding: Encoding::Spr2,
        version: set.version,
        frames,
    })
}

fn legacy_metadata(bytes: &[u8]) -> Result<Sprites> {
    ensure!(bytes.len() >= 8, "truncated legacy sprite directory");
    let first: [u8; 4] = bytes[..4].try_into().unwrap();
    let be = (500..=506).contains(&u32::from_be_bytes(first));
    let word = |b: [u8; 4]| {
        if be {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    };
    let short = |b: [u8; 2]| {
        if be {
            i16::from_be_bytes(b)
        } else {
            i16::from_le_bytes(b)
        }
    };
    let version = word(first);
    ensure!(
        (500..=506).contains(&version),
        "native legacy sprite metadata version unavailable"
    );
    let count = word(bytes[4..8].try_into().unwrap()) as usize;
    let start: usize = if version < 502 { 8 } else { 12 };
    let end = count
        .checked_mul(4)
        .and_then(|n| start.checked_add(n))
        .context("legacy sprite directory size overflow")?;
    ensure!(end <= bytes.len(), "truncated legacy sprite offsets");
    // 4c154e splits the raw frame vector into color/depth/(alpha) planes.
    let logical = count
        / match version {
            501 | 503 => 2,
            506 => 5,
            _ => 1,
        };
    let mut frames = Vec::with_capacity(logical);
    for n in 0..logical {
        let off = word(bytes[start + n * 4..start + n * 4 + 4].try_into().unwrap()) as usize;
        let end = off.checked_add(8).context("legacy frame bounds overflow")?;
        ensure!(off >= start + count * 4, "legacy frame overlaps directory");
        let b = bytes
            .get(off..end)
            .context("legacy frame bounds outside resource")?;
        frames.push(std::array::from_fn(|i| {
            short(b[i * 2..i * 2 + 2].try_into().unwrap())
        }));
    }
    Ok(Sprites {
        encoding: Encoding::Legacy,
        version,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_graphic_interval_retains_native_wrap_and_empty_cases() {
        assert_eq!(
            graphic_ids(128, 6).unwrap(),
            vec![128, 129, 130, 131, 132, 133]
        );
        assert!(graphic_ids(128, 0).unwrap().is_empty());
        assert!(graphic_ids(32760, 10).unwrap().is_empty());
        assert!(graphic_ids(0, -32768).is_err());
        assert_eq!(graphic_ids(-3, 2).unwrap(), vec![-3, -2]);
    }
    #[test]
    fn view_selection_uses_first_mask_match_and_signed_zoom() {
        let group = DrawGroup {
            version: 20004,
            views: vec![
                DrawView {
                    direction: 0x55,
                    zoom: 2,
                    sprites: vec![],
                },
                DrawView {
                    direction: 4,
                    zoom: 2,
                    sprites: vec![],
                },
                DrawView {
                    direction: 4,
                    zoom: u32::MAX,
                    sprites: vec![],
                },
            ],
        };
        assert!(std::ptr::eq(view(&group, 2, 2).unwrap(), &group.views[0]));
        assert!(std::ptr::eq(view(&group, 34, -1).unwrap(), &group.views[2]));
        assert!(view(&group, 1, 2).is_none());
    }
    #[test]
    fn raw_spr2_bounds_ignore_pixel_offsets_and_preserve_signed_dimensions() {
        let mut bytes = vec![];
        for n in [1000u32, 1, 7, 16] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend(65535u16.to_le_bytes());
        bytes.extend(3u16.to_le_bytes());
        bytes.extend([0; 8]);
        bytes.extend(20i16.to_le_bytes());
        bytes.extend(30i16.to_le_bytes());
        let s = spr2_metadata(&bytes).unwrap();
        assert_eq!(s.frames, [[0, 0, 3, -1]]);
    }
    #[test]
    fn legacy_metadata_keeps_color_plane_count_and_signed_rectangle() {
        for version in 500..=506u32 {
            for be in [false, true] {
                let planes = match version {
                    501 | 503 => 2,
                    506 => 5,
                    _ => 1,
                };
                let count = 2 * planes;
                let start = if version < 502 { 8 } else { 12 };
                let mut bytes = vec![];
                let word = |n: u32| if be { n.to_be_bytes() } else { n.to_le_bytes() };
                bytes.extend(word(version));
                bytes.extend(word(count));
                if version >= 502 {
                    bytes.extend(word(7));
                }
                for i in 0..count {
                    bytes.extend(word(start + 4 * count + 8 * i));
                }
                for _ in 0..count {
                    for n in [-3i16, 5, 12, 40] {
                        bytes.extend(if be { n.to_be_bytes() } else { n.to_le_bytes() });
                    }
                }
                let s = legacy_metadata(&bytes).unwrap();
                assert_eq!(s.frames, [[-3, 5, 12, 40]; 2]);
            }
        }
    }
}
