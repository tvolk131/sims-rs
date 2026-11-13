//! Owned software image endpoint for native portrait generator calls. The
//! native engine delegates stretching to DirectDraw. Our endpoint uses point
//! sampling after the verified 5dee3e clipping; driver-specific stretch pixels
//! and lost-surface recovery are not claimed to be native pixel equivalents.
pub mod thumbnail;
use super::{
    blit,
    resize::{Format, Image},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use crate::{
    Index,
    dialog::portrait::{
        Rect,
        generators::{Backend, Render, Rendered},
    },
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raster {
    pub format: Format,
    pub image: Image,
    pub key: Option<u16>,
}
impl Raster {
    /// Decode mounted BMP pixels into the selected display packing. BMP file
    /// decoding is supplied by the image library; native cache decisions and
    /// image ownership are handled by the portrait retrieval controller.
    pub fn from_bmp(bytes: &[u8], format: Format) -> Result<Self> {
        let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Bmp)?.to_rgba8();
        let result = Self {
            format,
            key: None,
            image: Image {
                width: rgba.width() as usize,
                height: rgba.height() as usize,
                pixels: rgba
                    .pixels()
                    .map(|p| format.encode([p[0], p[1], p[2]]))
                    .collect(),
                alpha: None,
            },
        };
        result.validate()?;
        Ok(result)
    }
    /// Native portrait persistence uses the full opaque RGB surface. The
    /// color-key and optional alpha plane are not consulted by 4b7cf4/4b7d93.
    pub fn bmp(&self) -> Result<Vec<u8>> {
        self.validate()?;
        super::bmp::encode(
            self.format,
            super::resize::Source {
                pixels: &self.image.pixels,
                width: self.image.width,
                height: self.image.height,
                stride: self.image.width,
                key: self.key,
            },
            None,
            false,
            0,
        )
        .map_err(anyhow::Error::msg)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.image.width > 0
                && self.image.height > 0
                && self.image.width <= i32::MAX as usize
                && self.image.height <= i32::MAX as usize,
            "invalid portrait image dimensions"
        );
        let size = self
            .image
            .width
            .checked_mul(self.image.height)
            .context("portrait image size overflow")?;
        ensure!(
            self.image.pixels.len() == size
                && self.image.alpha.as_ref().is_none_or(|a| a.len() == size),
            "portrait image storage mismatch"
        );
        Ok(())
    }
    pub fn rect(&self) -> Rect {
        [0, 0, self.image.width as i32, self.image.height as i32]
    }
    pub fn rgba(&self) -> Result<image::RgbaImage> {
        self.validate()?;
        Ok(image::RgbaImage::from_fn(
            self.image.width as u32,
            self.image.height as u32,
            |x, y| {
                let index = y as usize * self.image.width + x as usize;
                let p = self.image.pixels[index];
                let [r, g, b] = self.format.decode(p);
                let alpha = if self.key == Some(p) {
                    0
                } else {
                    self.image.alpha.as_ref().map_or(255, |a| a[index])
                };
                image::Rgba([r, g, b, alpha])
            },
        ))
    }
}

/// Bind the verified original Res_CPanel/Res_Other header and RT mappings.
/// Ambiguous/missing files remain explicit failures; no replacement artwork.
pub fn template(index: &Index, resource: u16, format: Format) -> Result<Option<Raster>> {
    let name = match resource {
        108 => "CPanel/People/ThumbBack.BMP",
        4300 => "CPanel/Catalog/ThumbTemplate.bmp",
        4600 => "CPanel/People/PeopleTemplate.bmp",
        4602 => "CPanel/People/WebPeopleTemplate.bmp",
        9000 => "Other/SpeechMedium.bmp",
        9001 => "Other/SpeechLarge.bmp",
        _ => return Ok(None),
    };
    let candidates = index
        .resources
        .iter()
        .filter(|r| r.name.replace('\\', "/").eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    ensure!(
        candidates.len() == 1,
        "portrait template {resource} has {} matching resources",
        candidates.len()
    );
    Ok(Some(Raster::from_bmp(&candidates[0].read()?, format)?))
}

struct Owner {
    refs: u32,
    pixels: Option<Raster>,
}
pub struct Software<F> {
    format: Format,
    templates: BTreeMap<u16, Raster>,
    draw: F,
    owners: Vec<Owner>,
    output: Option<usize>,
    error: Option<String>,
}
impl<F> Software<F> {
    pub fn new(
        format: Format,
        templates: BTreeMap<u16, Raster>,
        initial: Option<Raster>,
        draw: F,
    ) -> Result<Self> {
        for raster in templates.values().chain(initial.iter()) {
            raster.validate()?;
            ensure!(
                raster.format == format,
                "portrait template format differs from display"
            );
        }
        let owners = initial
            .map(|pixels| {
                vec![Owner {
                    refs: 1,
                    pixels: Some(pixels),
                }]
            })
            .unwrap_or_default();
        let output = (!owners.is_empty()).then_some(0);
        Ok(Self {
            format,
            templates,
            draw,
            owners,
            output,
            error: None,
        })
    }
    fn fail(&mut self, message: impl ToString) {
        if self.error.is_none() {
            self.error = Some(message.to_string());
        }
    }
    fn owner(&self, image: usize) -> Result<&Raster> {
        self.owners
            .get(image)
            .and_then(|o| o.pixels.as_ref())
            .context("portrait image absent")
    }
    pub fn finish(self, success: bool) -> Result<Rendered<Raster>> {
        if let Some(error) = self.error {
            return Err(anyhow::anyhow!(error));
        }
        for (i, owner) in self.owners.iter().enumerate() {
            ensure!(
                owner.refs == u32::from(self.output == Some(i)),
                "portrait image ownership leaked"
            );
            ensure!(
                self.output == Some(i) || owner.pixels.is_none(),
                "unpublished portrait allocation leaked"
            );
        }
        let image = self.output.map(|id| self.owner(id).cloned()).transpose()?;
        Ok(Rendered { success, image })
    }
    fn put(&mut self, pixels: Option<Raster>, refs: u32) -> usize {
        let id = self.owners.len();
        self.owners.push(Owner { refs, pixels });
        id
    }
}

/// Native 4e7082 control flow over the same owned image endpoint. Inputs are
/// borrowed; local wrappers keep their copies alive until composition finishes.
pub fn compose(
    layout: crate::dialog::portrait::Layout,
    faces: &Raster,
    background: &Raster,
    key: u32,
) -> Result<Rendered<Raster>> {
    use crate::dialog::portrait as p;
    ensure!(
        faces.format == background.format,
        "portrait composition format mismatch"
    );
    let draw: fn(Render) -> Result<Rendered<Image>> =
        |_| anyhow::bail!("composition unexpectedly requested a head draw");
    struct Composer {
        host: Software<fn(Render) -> Result<Rendered<Image>>>,
        faces: usize,
        background: usize,
        destination: Option<usize>,
    }
    impl p::Backend for Composer {
        fn allocate(&mut self) -> bool {
            self.destination = Backend::allocate(&mut self.host);
            self.destination.is_some()
        }
        fn create(&mut self, w: i32, h: i32) -> bool {
            Backend::create(&mut self.host, self.destination.unwrap(), w, h)
        }
        fn destroy(&mut self) {
            Backend::destroy(&mut self.host, self.destination.unwrap());
        }
        fn lock(&mut self, mode: u32) -> bool {
            Backend::lock(&mut self.host, self.destination.unwrap(), mode)
        }
        fn unlock(&mut self, mode: u32) {
            Backend::unlock(&mut self.host, self.destination.unwrap(), mode);
        }
        fn fill(&mut self, key: u32) -> bool {
            let raster = self.host.owners[self.destination.unwrap()]
                .pixels
                .as_mut()
                .unwrap();
            raster.image.pixels.fill(key as u16);
            true
        }
        fn color_key(&mut self, key: u32) {
            Backend::color_key(&mut self.host, self.destination.unwrap(), key);
        }
        fn blit(&mut self, source: p::Source, from: Rect, to: Rect) -> bool {
            let source = match source {
                p::Source::Faces => self.faces,
                p::Source::Background => self.background,
            };
            Backend::blit(&mut self.host, self.destination.unwrap(), source, from, to)
        }
        fn publish(&mut self) {
            Backend::publish(&mut self.host, self.destination.unwrap());
        }
    }
    faces.validate()?;
    background.validate()?;
    let mut host = Software::new(faces.format, BTreeMap::new(), None, draw)?;
    let face_id = host.put(Some(faces.clone()), 1);
    let background_id = host.put(Some(background.clone()), 1);
    let mut backend = Composer {
        host,
        faces: face_id,
        background: background_id,
        destination: None,
    };
    let success = p::compose(layout, faces.rect(), background.rect(), key, &mut backend);
    Backend::release(&mut backend.host, face_id);
    Backend::release(&mut backend.host, background_id);
    backend.host.finish(success)
}
impl<F: FnMut(Render) -> Result<Rendered<Image>>> Backend for Software<F> {
    type Image = usize;
    fn allocate(&mut self) -> Option<usize> {
        if let Err(e) = self.owners.try_reserve(1) {
            self.fail(e);
            return None;
        }
        Some(self.put(None, 0))
    }
    fn create(&mut self, id: usize, width: i32, height: i32) -> bool {
        if width <= 0 || height <= 0 {
            return false;
        }
        let Some(count) = (width as usize).checked_mul(height as usize) else {
            return false;
        };
        let mut pixels = Vec::new();
        if let Err(e) = pixels.try_reserve_exact(count) {
            self.fail(e);
            return false;
        }
        pixels.resize(count, 0);
        self.owners[id].pixels = Some(Raster {
            format: self.format,
            key: None,
            image: Image {
                width: width as usize,
                height: height as usize,
                pixels,
                alpha: None,
            },
        });
        true
    }
    fn destroy(&mut self, id: usize) {
        if self.owners[id].refs != 0 {
            self.fail("destroyed referenced portrait image");
            return;
        }
        self.owners[id].pixels = None;
    }
    fn template(&mut self, resource: u16) -> Option<usize> {
        let raster = self.templates.get(&resource)?.clone();
        Some(self.put(Some(raster), 0))
    }
    fn lock(&mut self, id: usize, _: u32) -> bool {
        self.owner(id).is_ok()
    }
    fn unlock(&mut self, _: usize, _: u32) {}
    fn color_key(&mut self, id: usize, color: u32) {
        // 4ff31a initializes 6f2ab4 through the display's RGB packer; 5dfd3a
        // stores that already-packed word. Do not treat it as RGB888 again.
        let key = color as u16;
        match self.owners[id].pixels.as_mut() {
            Some(r) => r.key = Some(key),
            None => self.fail("keyed absent portrait image"),
        }
    }
    fn first_pixel(&mut self, id: usize, _: bool) -> u16 {
        self.owner(id).map(|r| r.image.pixels[0]).unwrap_or(0)
    }
    fn rect(&self, id: usize) -> Rect {
        self.owner(id).map_or([0; 4], Raster::rect)
    }
    fn render(&mut self, request: Render) -> Rendered<usize> {
        match (self.draw)(request) {
            Ok(rendered) => {
                let image = rendered.image.map(|image| {
                    self.put(
                        Some(Raster {
                            format: self.format,
                            key: None,
                            image,
                        }),
                        1,
                    )
                });
                Rendered {
                    success: rendered.success,
                    image,
                }
            }
            Err(e) => {
                self.fail(e);
                Rendered {
                    success: false,
                    image: None,
                }
            }
        }
    }
    fn blit(&mut self, destination: usize, source: usize, from: Rect, to: Rect) -> bool {
        let result = (|| -> Result<()> {
            let source = self.owner(source)?.clone();
            source.validate()?;
            let destination = self
                .owners
                .get_mut(destination)
                .and_then(|o| o.pixels.as_mut())
                .context("portrait blit destination absent")?;
            destination.validate()?;
            ensure!(
                source.format == destination.format,
                "portrait blit format conversion unsupported"
            );
            ensure!(
                source.image.alpha.is_none(),
                "portrait alpha-plane blit unsupported"
            );
            let Some((from, to)) = blit::rectangles(
                [
                    destination.image.width as i32,
                    destination.image.height as i32,
                ],
                from,
                to,
                None,
            ) else {
                return Ok(());
            };
            ensure!(
                from[0] >= 0
                    && from[1] >= 0
                    && from[2] > from[0]
                    && from[3] > from[1]
                    && from[2] as usize <= source.image.width
                    && from[3] as usize <= source.image.height,
                "portrait blit source rectangle invalid"
            );
            let (sw, sh, dw, dh) = (
                (from[2] - from[0]) as usize,
                (from[3] - from[1]) as usize,
                (to[2] - to[0]) as usize,
                (to[3] - to[1]) as usize,
            );
            for y in 0..dh {
                for x in 0..dw {
                    let sx = from[0] as usize + x * sw / dw;
                    let sy = from[1] as usize + y * sh / dh;
                    let p = source.image.pixels[sy * source.image.width + sx];
                    if source.key != Some(p) {
                        destination.image.pixels
                            [(to[1] as usize + y) * destination.image.width + to[0] as usize + x] =
                            p;
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.fail(e);
            false
        } else {
            true
        }
    }
    fn release(&mut self, id: usize) {
        let Some(count) = self.owners[id].refs.checked_sub(1) else {
            self.fail("portrait reference underflow");
            return;
        };
        self.owners[id].refs = count;
        if count == 0 {
            self.destroy(id);
        }
    }
    fn publish(&mut self, id: usize) {
        let Some(count) = self.owners[id].refs.checked_add(1) else {
            self.fail("portrait reference overflow");
            return;
        };
        self.owners[id].refs = count;
        if let Some(old) = self.output {
            self.release(old);
        }
        self.output = Some(id);
    }
    fn output(&self) -> Option<usize> {
        self.output
    }
}
