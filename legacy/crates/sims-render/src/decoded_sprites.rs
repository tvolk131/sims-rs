use crate::Texture;
use anyhow::{Result, ensure};
use sims_formats::sprite::Frame;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Documents {
    entries: BTreeMap<String, DocumentFrames>,
}

#[derive(Default)]
struct DocumentFrames {
    bytes: Option<Vec<u8>>,
    frames: Frames,
}

impl Documents {
    pub(super) fn for_document(&mut self, source: &str, bytes: &[u8]) -> &mut Frames {
        let document = self.entries.entry(source.to_owned()).or_default();
        if document.bytes.as_deref() != Some(bytes) {
            document.bytes = None;
            document.frames = Frames::default();
        }
        &mut document.frames
    }

    pub(super) fn retain_document(&mut self, source: &str, bytes: Vec<u8>) {
        self.entries
            .get_mut(source)
            .expect("decoded sprite document was not opened")
            .bytes = Some(bytes);
    }
}

#[derive(Default)]
pub(super) struct Frames {
    textures: BTreeMap<(u16, usize), Option<Texture>>,
}

impl Frames {
    pub(super) fn get(
        &mut self,
        key: (u16, usize),
        decode: impl FnOnce() -> Result<Frame>,
    ) -> Result<Option<Texture>> {
        if let Some(texture) = self.textures.get(&key) {
            return Ok(texture.clone());
        }
        let frame = decode()?;
        let texture = if frame.width == 0 || frame.height == 0 {
            None
        } else {
            ensure!(
                frame.flags & 2 != 0,
                "object sprite needs original depth channel"
            );
            Some(Texture {
                width: frame.width,
                height: frame.height,
                rgba: frame.rgba.into(),
                depth: frame.depth.into(),
            })
        };
        self.textures.insert(key, texture.clone());
        Ok(texture)
    }
}

#[cfg(test)]
mod tests;
