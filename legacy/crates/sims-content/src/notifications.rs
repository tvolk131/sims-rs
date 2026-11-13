//! Retained CPU surfaces for native person notifications. Control flow is in
//! sims-sim; COSMIC Text supplies explicit portable font/surface services. This
//! renderer does not claim identical Windows font metrics or DirectDraw pixels.
use anyhow::{Context, Result, ensure};
use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap, fontdb,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_sim::person_notification::{
    Style,
    currency::Locale,
    display::{Cache, Request, Response},
};
use sims_vm::ObjectId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Font {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub face: u32,
    pub family: String,
    pub pixels: u16,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub created: bool,
    pub locked: bool,
    pub text: Vec<u8>,
    pub draws: u64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub cache: Option<Cache>,
    pub surface: Option<Surface>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub font: Font,
    pub locale: Locale,
    pub font_color: u32,
    pub entries: BTreeMap<ObjectId, Entry>,
}
pub struct Software {
    pub saved: Saved,
    fonts: FontSystem,
    swash: SwashCache,
}

impl Software {
    /// Choose an application font once, then retain its bytes in every saved
    /// presenter. This is an explicit UI choice, not a substitute game asset.
    pub fn system_font(locale: Locale) -> Result<Self> {
        let fonts = FontSystem::new();
        let id = fonts
            .db()
            .query(&fontdb::Query {
                families: &[
                    fontdb::Family::Name("Arial"),
                    fontdb::Family::Name("Liberation Sans"),
                    fontdb::Family::SansSerif,
                ],
                ..Default::default()
            })
            .context("no notification application font available")?;
        let family = fonts
            .db()
            .face(id)
            .context("notification font face absent")?
            .families
            .first()
            .context("notification font family absent")?
            .0
            .clone();
        let (bytes, face) = fonts
            .db()
            .with_face_data(id, |data, index| (data.to_vec(), index))
            .context("read notification font bytes")?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        Self::restore(Saved {
            font: Font {
                bytes,
                sha256,
                face,
                family,
                pixels: 12,
            },
            locale,
            font_color: 0x000000ff,
            entries: BTreeMap::new(),
        })
    }
    pub fn restore(saved: Saved) -> Result<Self> {
        ensure!(
            saved.font.bytes.len() <= 64 * 1024 * 1024
                && format!("{:x}", Sha256::digest(&saved.font.bytes)) == saved.font.sha256,
            "notification font identity changed"
        );
        ensure!(
            (1..=256).contains(&saved.font.pixels),
            "invalid notification font size"
        );
        let mut db = fontdb::Database::new();
        db.load_font_data(saved.font.bytes.clone());
        let keep = db
            .faces()
            .find(|face| face.index == saved.font.face)
            .context("saved notification font face absent")?
            .id;
        ensure!(
            db.face(keep)
                .unwrap()
                .families
                .iter()
                .any(|(name, _)| name == &saved.font.family),
            "saved notification font family changed"
        );
        let remove: Vec<_> = db
            .faces()
            .filter(|face| face.id != keep)
            .map(|face| face.id)
            .collect();
        for id in remove {
            db.remove_face(id);
        }
        let fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
        let result = Self {
            saved,
            fonts,
            swash: SwashCache::new(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        for (&id, entry) in &self.saved.entries {
            ensure!(id > 0, "invalid notification owner");
            let bounds = entry.cache.as_ref().and_then(|c| c.surface);
            ensure!(
                bounds.is_some() == entry.surface.is_some(),
                "notification surface owner lost"
            );
            if let Some(surface) = &entry.surface {
                ensure!(
                    surface.width <= 4096
                        && surface.height <= 4096
                        && u64::from(surface.width) * u64::from(surface.height) <= 4_194_304,
                    "invalid notification surface size"
                );
                ensure!(
                    surface.rgba.len()
                        == if surface.created {
                            surface.width as usize * surface.height as usize * 4
                        } else {
                            0
                        },
                    "notification pixel storage changed"
                );
                ensure!(
                    bounds == Some([0, 0, surface.width as i32, surface.height as i32]),
                    "notification cache and surface bounds differ"
                );
                ensure!(
                    !surface.locked || surface.created,
                    "unallocated notification lock"
                );
            }
        }
        Ok(())
    }
    /// RGB values recovered from 4ff31a and the two numeric display wrappers.
    /// The CPU backend uses RGBA8 rather than the native 16-bit pixel packing.
    pub fn colors(style: Style) -> [u32; 2] {
        match style {
            Style::Default => [0x000052ff, 0x00ca39ff],
            Style::Yellow => [0xffcb17ff, 0x000000ff],
            Style::Magenta => [0xff00ffff, 0x000000ff],
        }
    }
    fn buffer(&mut self, text: &[u8]) -> Result<Buffer> {
        ensure!(
            !text.contains(&0) && text.len() <= 16384,
            "invalid notification text"
        );
        let text = sims_formats::strings::decode(text, self.saved.locale.language.try_into()?);
        let size = f32::from(self.saved.font.pixels);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size + 2.));
        buffer.set_wrap(&mut self.fonts, Wrap::None);
        buffer.set_size(&mut self.fonts, None, None);
        buffer.set_text(
            &mut self.fonts,
            &text,
            &Attrs::new().family(Family::Name(&self.saved.font.family)),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts, true);
        Ok(buffer)
    }
    pub fn service(&mut self, id: ObjectId, request: &Request, now: u32) -> Result<Response> {
        use Request as R;
        Ok(match request {
            R::Measure { text } => {
                let buffer = self.buffer(text)?;
                let mut size = [0f32; 2];
                for run in buffer.layout_runs() {
                    size[0] = size[0].max(run.line_w);
                    size[1] = size[1].max(run.line_top + run.line_height);
                }
                Response::Bounds([0, 0, size[0].ceil() as i32, size[1].ceil() as i32])
            }
            R::Allocate => {
                let entry = self
                    .saved
                    .entries
                    .get_mut(&id)
                    .context("notification entry absent")?;
                ensure!(
                    entry.surface.is_none(),
                    "notification surface already allocated"
                );
                entry.surface = Some(Surface::default());
                Response::Bounds([0; 4])
            }
            R::Create { size } => {
                ensure!(
                    size.iter().all(|n| (1..=4096).contains(n))
                        && i64::from(size[0]) * i64::from(size[1]) <= 4_194_304,
                    "notification surface exceeds renderer limits"
                );
                let s = self.surface_mut(id)?;
                ensure!(!s.locked, "recreating locked notification surface");
                s.width = size[0] as u32;
                s.height = size[1] as u32;
                s.rgba = vec![0; s.width as usize * s.height as usize * 4];
                s.created = true;
                s.text.clear();
                Response::Created {
                    bounds: [0, 0, size[0], size[1]],
                    success: true,
                }
            }
            R::Release => {
                let s = self.surface_mut(id)?;
                ensure!(!s.locked, "releasing locked notification surface");
                s.rgba.clear();
                s.created = false;
                s.text.clear();
                Response::Done
            }
            R::Clock => Response::Clock(now),
            R::Lock => {
                let s = self.surface_mut(id)?;
                ensure!(s.created && !s.locked, "notification lock unavailable");
                s.locked = true;
                Response::Bool(true)
            }
            R::Fill { color, rect } => {
                let s = self.surface_mut(id)?;
                ensure!(s.locked, "notification fill without lock");
                for y in rect[1].max(0)..rect[3].min(s.height as i32) {
                    for x in rect[0].max(0)..rect[2].min(s.width as i32) {
                        let offset = (y as usize * s.width as usize + x as usize) * 4;
                        s.rgba[offset..offset + 4].copy_from_slice(&color.to_be_bytes());
                    }
                }
                Response::Bool(true)
            }
            R::Unlock => {
                let s = self.surface_mut(id)?;
                ensure!(s.locked, "notification unlock without lock");
                s.locked = false;
                Response::Done
            }
            R::FontColor => Response::Color(self.saved.font_color),
            R::SetFontColor { color } => {
                self.saved.font_color = *color;
                Response::Done
            }
            R::Draw { text, x, y } => {
                let buffer = self.buffer(text)?;
                let [r, g, b, a] = self.saved.font_color.to_be_bytes();
                let s = self
                    .saved
                    .entries
                    .get_mut(&id)
                    .and_then(|e| e.surface.as_mut())
                    .context("notification draw surface absent")?;
                ensure!(
                    s.created && !s.locked,
                    "notification draw surface unavailable"
                );
                buffer.draw(
                    &mut self.fonts,
                    &mut self.swash,
                    Color::rgba(r, g, b, a),
                    |px, py, w, h, color| {
                        let color = color.as_rgba();
                        for row in 0..h as i32 {
                            for col in 0..w as i32 {
                                let dx = px + x + col;
                                let dy = py + y + row;
                                if dx < 0 || dy < 0 || dx >= s.width as i32 || dy >= s.height as i32
                                {
                                    continue;
                                }
                                let dst = &mut s.rgba
                                    [(dy as usize * s.width as usize + dx as usize) * 4..][..4];
                                let alpha = u32::from(color[3]);
                                for channel in 0..3 {
                                    dst[channel] = ((u32::from(color[channel]) * alpha
                                        + u32::from(dst[channel]) * (255 - alpha)
                                        + 127)
                                        / 255)
                                        as u8;
                                }
                                dst[3] = (alpha + u32::from(dst[3]) * (255 - alpha) / 255) as u8;
                            }
                        }
                    },
                );
                s.text = text.clone();
                s.draws += 1;
                Response::Done
            }
        })
    }
    fn surface_mut(&mut self, id: ObjectId) -> Result<&mut Surface> {
        self.saved
            .entries
            .get_mut(&id)
            .and_then(|e| e.surface.as_mut())
            .context("notification surface absent")
    }
    /// Native blitting uses the cached source rectangle, not the whole surface
    /// capacity. A shorter message must not expose pixels left outside its fill.
    pub fn image(&self, id: ObjectId) -> Result<image::RgbaImage> {
        let entry = self
            .saved
            .entries
            .get(&id)
            .context("notification entry absent")?;
        let cache = entry.cache.as_ref().context("notification cache absent")?;
        let s = entry
            .surface
            .as_ref()
            .context("notification surface absent")?;
        ensure!(s.created && !s.locked, "notification pixels unavailable");
        let [left, top, right, bottom] = cache.rect;
        ensure!(
            left >= 0
                && top >= 0
                && right > left
                && bottom > top
                && right <= s.width as i32
                && bottom <= s.height as i32,
            "notification source rectangle outside surface"
        );
        let pixels = image::RgbaImage::from_raw(s.width, s.height, s.rgba.clone())
            .context("invalid notification pixels")?;
        Ok(image::imageops::crop_imm(
            &pixels,
            left as u32,
            top as u32,
            (right - left) as u32,
            (bottom - top) as u32,
        )
        .to_image())
    }
    /// Call for people included in a rendered frame, after synchronous popup
    /// services finish. Original 437e9c uses unsigned age and removes at 100.
    pub fn present(&mut self, id: ObjectId, now: u32) -> Option<&Surface> {
        let entry = self.saved.entries.get(&id)?;
        let cache = entry.cache.as_ref()?;
        if !sims_sim::person_notification::display::within_display_lifetime(cache.clock, now) {
            self.saved.entries.remove(&id);
            return None;
        }
        self.saved
            .entries
            .get(&id)?
            .surface
            .as_ref()
            .filter(|s| s.created)
    }
}
