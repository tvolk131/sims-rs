//! Resident first-floor SPR# draw. Other variants retain the pending caller.
use super::*;
use sims_sim::lot_presenter::{sprite_full as raster, terrain_raster::Data};
const SOURCE: &str = "321937b4f024bd44a5d6717cef92f83bcf10c1f7c6b6a011835a11c6a2e2513f";
const SHADED: &str = "09e69c759da119c185d87ba32eb34a9bc3f584ceec5898b8b40143e9621af861";
const CATALOG: &str = "dacd8b0d097cc774835e28ef5335baca39241c5fd3c0ed6052ba3a86d221aedb";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Shade {
    header: Vec<u8>,
    threshold: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Palettes {
    header: Vec<u8>,
    threshold: u32,
    working: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    source: String,
    pub image: u32,
    pub palette_owner: u32,
    resource: u32,
    resource_raw: Vec<u8>,
    frame_address: u32,
    frame: Vec<u8>,
    color_owner: u32,
    palette: Vec<u8>,
    rows_address: u32,
    origin_y: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shade: Option<Shade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    palettes: Option<Palettes>,
}
impl Source {
    pub fn import(input: &[u8]) -> Result<Self> {
        let digest = sims_fixtures::digest(input);
        ensure!(
            digest == SOURCE || digest == SHADED,
            "floor sprite source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        Self::decode(&v, &v["sprite"], digest)
    }
    pub fn import_catalog(input: &[u8]) -> Result<Vec<Self>> {
        let digest = sims_fixtures::digest(input);
        ensure!(digest == CATALOG, "resident floor catalog differs");
        let v: serde_json::Value = serde_json::from_slice(input)?;
        let records = v["sprites"]
            .as_array()
            .context("resident floor records absent")?;
        ensure!(records.len() == 9, "resident floor count differs");
        records
            .iter()
            .map(|meta| Self::decode(&v, meta, digest.clone()))
            .collect()
    }
    fn decode(v: &serde_json::Value, meta: &serde_json::Value, digest: String) -> Result<Self> {
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "floor sprite epoch differs"
        );
        let m = Memory::new(v)?;
        let image = meta["image"]
            .as_u64()
            .context("sprite source image absent")? as u32;
        let palette_owner = meta["palette_owner"]
            .as_u64()
            .context("sprite source palette absent")? as u32;
        let owner = m.read(palette_owner, 0x2c)?;
        let lo = word(&owner, 8);
        let hi = word(&owner, 12);
        ensure!(hi > lo && (hi - lo) % 4 == 0, "sprite cache buckets differ");
        let mut p = word(&m.read(lo + (image % ((hi - lo) / 4)) * 4, 4)?, 0);
        let mut seen = std::collections::BTreeSet::new();
        let mut resource = None;
        while p != 0 {
            ensure!(
                seen.insert(p) && seen.len() < 2048,
                "sprite resident chain differs"
            );
            let n = m.read(p, 12)?;
            if word(&n, 4) == image {
                resource = Some(word(&n, 8));
                break;
            }
            p = word(&n, 0);
        }
        let resource = resource.context("sprite was not resident at B")?;
        let resource_raw = m.read(resource, 0x2c)?;
        let chunk = word(&resource_raw, 12);
        let header = m.read(chunk, 16)?;
        ensure!(
            word(&header, 0) == 1000 && word(&header, 4) == 1,
            "sprite format/frame count differs"
        );
        let frame_address = chunk + word(&header, 12);
        let frame = m.read(
            frame_address,
            meta["frame_raw"]
                .as_str()
                .context("sprite frame size absent")?
                .len()
                / 2,
        )?;
        let color_owner = word(&resource_raw, 8);
        let shade = if digest == SHADED {
            let header = m.read(color_owner, 32)?;
            let threshold = word(&m.read(0x6f0180, 12)?, 4);
            ensure!(
                word(&header, 24) == 0.5f32.to_bits() && word(&header, 28) >= threshold,
                "shaded sprite needs native palette rebuild/conversion"
            );
            Some(Shade { header, threshold })
        } else {
            None
        };
        let palette = m.read(
            color_owner + if shade.is_some() { 0xc20 } else { 0x420 },
            512,
        )?;
        let palettes = if digest == CATALOG {
            Some(Palettes {
                header: m.read(color_owner, 32)?,
                threshold: word(&m.read(0x6f0180, 12)?, 4),
                working: m.read(color_owner + 0xc20, 512)?,
            })
        } else {
            None
        };
        let projection = m.read(0x6f00d0, 16)?;
        let zoom = word(&projection, 8);
        ensure!(zoom == 1, "sprite source zoom differs");
        let rows_address = word(&m.read(0x09e88960 + 0xb8, 0xb0)?, (7 - zoom) as usize * 4);
        let planes = m.read(0x6f0c0c, 28)?;
        ensure!(
            word(&planes, 0) == 1600 && word(&planes, 24) == 1600,
            "sprite baseline plane pitch differs"
        );
        let s = Self {
            source: digest,
            image,
            palette_owner,
            resource,
            resource_raw,
            frame_address,
            frame,
            color_owner,
            palette,
            rows_address,
            origin_y: word(&projection, 4) as i32,
            shade,
            palettes,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (self.source == SOURCE && self.image == 27 && self.shade.is_none()
                || self.source == SHADED
                    && self.image == 2
                    && self.shade.as_ref().is_some_and(|s| s.header.len() == 32
                        && word(&s.header, 24) == 0.5f32.to_bits()
                        && word(&s.header, 28) >= s.threshold)
                || self.source == CATALOG
                    && self.shade.is_none()
                    && self.palettes.as_ref().is_some_and(|p| p.header.len() == 32
                        && p.working.len() == 512
                        && f32::from_bits(word(&p.header, 24)).is_finite()))
                && self.resource_raw.len() == 44
                && self.frame.len() >= 18
                && self.frame[..4] == [31, 0, 16, 0]
                && self.palette.len() == 512
                && self.frame[12..16] == [0; 4],
            "sprite resident inputs differ"
        );
        Ok(())
    }
    fn light(&self) -> u32 {
        self.shade
            .as_ref()
            .map_or(1f32.to_bits(), |s| word(&s.header, 24))
    }
    fn working(&self, light: Option<u32>) -> bool {
        self.shade.is_some() || self.palettes.is_some() && light != Some(1f32.to_bits())
    }
    fn palette_address(&self, light: Option<u32>) -> u32 {
        self.color_owner + if self.working(light) { 0xc20 } else { 0x420 }
    }
    fn colors(&self, light: Option<u32>) -> &[u8] {
        if self.working(light)
            && let Some(p) = &self.palettes
        {
            &p.working
        } else {
            &self.palette
        }
    }
    fn cache(&self) -> Option<(&[u8], u32, &[u8])> {
        self.palettes
            .as_ref()
            .map(|p| (p.header.as_slice(), p.threshold, p.working.as_slice()))
            .or_else(|| {
                self.shade
                    .as_ref()
                    .map(|s| (s.header.as_slice(), s.threshold, self.palette.as_slice()))
            })
    }
    fn supports_light(&self, light: u32) -> bool {
        if let Some(p) = &self.palettes {
            light == 1f32.to_bits()
                || light == word(&p.header, 24) && word(&p.header, 28) >= p.threshold
        } else {
            light == self.light()
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    point: [i8; 3],
    origin: [i32; 2],
    rotation: u8,
    stage: u8,
    rect: Option<[i32; 4]>,
    base: Option<i32>,
    rows: Vec<i32>,
    writes: Vec<raster::Write>,
    next: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    margins: Option<[i32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    palette_light: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gradient: Option<gradient::Cursor>,
}
impl Cursor {
    pub fn new(point: [i8; 3], origin: [i32; 2], rotation: u8) -> Self {
        Self {
            point,
            origin,
            rotation,
            stage: 0,
            rect: None,
            base: None,
            rows: vec![],
            writes: vec![],
            next: 0,
            margins: None,
            palette_light: None,
            gradient: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == 7
    }
    /// Original culling, depth preparation and palette getter precede the
    /// lighting branch. Fully clipped resources never reach this point.
    pub fn dispatch_pending(&self) -> bool {
        self.stage == 4
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        if let Some(g) = &self.gradient {
            if !g.complete() { return g.pc(self.margins.is_some()) == Some(pc); }
        }
        match pc {
            0x4c811f => self.stage == 0,
            0x4c8e0d => self.stage == 1,
            0x4c8f52 => self.stage == 2,
            0x4c8152 => self.stage == 3,
            0x49f42d => self.stage == 4,
            0x49f628 => self.stage == 5 && self.margins.is_none(),
            0x49f949 => self.stage == 5 && self.margins.is_some(),
            0x4b954a => self.stage == 8,
            0x4c82c9 => self.stage == 9,
            0x4b0000 => self.stage == 7,
            _ => false,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(g) = &self.gradient { g.validate()?; }
        ensure!(
            self.stage <= 9
                && self.rotation <= 3
                && self.next <= self.writes.len()
                && self.rect.is_some() == (self.stage >= 4)
                && (self.base.is_some() == (self.stage >= 4)
                    || self.stage == 7
                        && self.rect == Some([i32::MAX, i32::MAX, i32::MIN, i32::MIN]))
                && self
                    .margins
                    .is_none_or(|v| self.stage >= 4 && v.iter().all(|&m| m >= 0)),
            "sprite retained cursor differs"
        );
        Ok(())
    }
    pub(super) fn advance_lighting(&mut self, source: &Source, g: &gradient::Source,
        descriptor: [u8;4], radius: i32, zoom: u8, brightness: u32, data: &mut Data) -> Result<Option<bool>> {
        if self.stage != 4 || !g.enabled() { return Ok(None); }
        if self.gradient.is_none() {
            self.gradient = Some(gradient::Cursor::new(descriptor, radius, zoom, brightness));
            return Ok(Some(true));
        }
        let child = self.gradient.as_mut().unwrap();
        if child.complete() { return Ok(None); }
        let progress = child.advance(g, gradient::Draw {
            frame: &source.frame, frame_address: source.frame_address, image: source.image,
            origin: self.origin, base: self.base.context("gradient base absent")?,
            rows: &self.rows, margins: self.margins }, data)?;
        if child.gradient_done() { self.stage = 7; }
        Ok(Some(progress))
    }
    pub(super) fn compare_lighting(&self, g: &gradient::Source, source: &Source,
        row: &serde_json::Value, data: &Data) -> Result<()> {
        self.gradient.as_ref().context("gradient child absent")?.compare(g, gradient::Draw {
            frame: &source.frame, frame_address: source.frame_address, image: source.image,
            origin: self.origin, base: self.base.context("gradient base absent")?,
            rows: &self.rows, margins: self.margins }, row, data)
    }
    pub fn advance(
        &mut self,
        source: &Source,
        placement: &placement::Source,
        sim: &sims_sim::Simulation,
        data: &mut Data,
        context: &[u32; 29],
        clip: [i16; 4],
    ) -> Result<bool> {
        source.validate()?;
        self.validate()?;
        match self.stage {
            0 => self.stage = 1,
            1 => self.stage = 2,
            2 => self.stage = 3,
            3 => {
                ensure!(
                    source.supports_light(sims_sim::lot_presenter::floor_prep::brightness(context))
                        && context[13] & 0xffffff00 == 0
                        && context[14] & 255 == 0
                        && context[23] & 8 == 0
                        && context[22] & 3 == 3,
                    "sprite context requires a different native branch"
                );
                if source.palettes.is_some() {
                    self.palette_light =
                        Some(sims_sim::lot_presenter::floor_prep::brightness(context));
                }
                let width = i32::from(u16::from_le_bytes(source.frame[0..2].try_into().unwrap()));
                let height =
                    usize::from(u16::from_le_bytes(source.frame[2..4].try_into().unwrap()));
                let [x, y] = self.origin;
                let rect = [x, y, x + width, y + height as i32];
                //4c85bb compares signed-short T/L/B/R rectangles, publishing
                //L/T/R/B margins to49f42d. Disjoint sprites retain sentinel bounds.
                ensure!(
                    rect.iter().all(|&v| i16::try_from(v).is_ok()),
                    "sprite bounds exceed native short range"
                );
                if y >= i32::from(clip[2])
                    || x >= i32::from(clip[3])
                    || i32::from(clip[1]) >= rect[2]
                    || i32::from(clip[0]) >= rect[3]
                {
                    self.rect = Some([i32::MAX, i32::MAX, i32::MIN, i32::MIN]);
                    self.stage = 7;
                    return Ok(true);
                }
                let margins = [
                    (i32::from(clip[1]) - x).max(0),
                    (i32::from(clip[0]) - y).max(0),
                    (rect[2] - i32::from(clip[3])).max(0),
                    (rect[3] - i32::from(clip[2])).max(0),
                ];
                let (base, rows) =
                    placement.sprite_depth(sim, self.point, self.rotation, y, height)?;
                self.rect = Some([
                    x + margins[0],
                    y + margins[1],
                    rect[2] - margins[2],
                    rect[3] - margins[3],
                ]);
                self.margins = (margins != [0; 4]).then_some(margins);
                self.base = Some(base);
                self.rows = rows;
                self.stage = if source.shade.is_some() || source.palettes.is_some() {
                    8
                } else {
                    4
                };
            }
            4 => self.stage = 5,
            5 => {
                let input = raster::Input {
                    frame: &source.frame,
                    palette: source.colors(self.palette_light),
                    rows: &self.rows,
                    base: self.base.unwrap(),
                    x: self.origin[0],
                    y: self.origin[1],
                    color_pitch: data.color_pitch / 2,
                    depth_pitch: data.depth_pitch / 2,
                };
                self.writes = if let Some(clip) = self.margins {
                    sims_sim::lot_presenter::sprite_clipped::plan(input, clip)
                } else {
                    raster::plan(input)
                }
                .map_err(anyhow::Error::msg)?;
                self.stage = 6;
            }
            6 => {
                if let Some(w) = self.writes.get(self.next) {
                    raster::apply(w, &mut data.color, &mut data.depth)
                        .map_err(anyhow::Error::msg)?;
                    self.next += 1;
                } else {
                    self.stage = 7;
                }
            }
            8 => self.stage = 9,
            9 => self.stage = 4,
            _ => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn compare(&self, source: &Source, row: &serde_json::Value, data: &Data) -> Result<()> {
        self.validate()?;
        let pc = row["pc"].as_u64().context("sprite PC absent")? as u32;
        ensure!(self.at_native_cut(pc), "sprite cut differs");
        let stack = crate::original_session::completed_animations::bytes(
            row["stack"]["raw"]
                .as_str()
                .context("sprite stack absent")?,
        )?;
        if pc == 0x4c8e0d {
            ensure!(
                word(&stack, 4) == source.image && row["registers"]["ecx"] == source.palette_owner,
                "sprite resource call differs"
            );
        }
        if matches!(pc, 0x4c8f52 | 0x4c8152) {
            let r = &row["sprite_resource"];
            ensure!(
                r["pointer"] == source.resource
                    && r["frame"] == source.frame_address
                    && crate::original_session::completed_animations::bytes(
                        r["raw"].as_str().context("sprite resource absent")?
                    )? == source.resource_raw[..16],
                "sprite B resident result differs"
            );
        }
        if pc == 0x4b954a {
            let (header, threshold, working) = source.cache().context("shade owner absent")?;
            let n = &row["shade_owner"];
            ensure!(
                n["pointer"] == source.color_owner
                    && word(&stack, 4) == self.palette_light.unwrap_or_else(|| source.light())
                    && crate::original_session::completed_animations::bytes(
                        n["raw"].as_str().context("shade header absent")?
                    )? == header
                    && word(
                        &crate::original_session::completed_animations::bytes(
                            n["epoch"].as_str().context("shade epoch absent")?
                        )?,
                        0
                    ) == threshold
                    && crate::original_session::completed_animations::bytes(
                        n["working"].as_str().context("shade table absent")?
                    )? == working,
                "shade getter cache inputs differ"
            );
        }
        if pc == 0x4c82c9 {
            let (header, threshold, _) = source.cache().context("shade owner absent")?;
            let n = &row["shade_result"];
            ensure!(
                n["pointer"] == source.palette_address(self.palette_light)
                    && crate::original_session::completed_animations::bytes(
                        n["raw"].as_str().context("shade result absent")?
                    )? == source.colors(self.palette_light)
                    && crate::original_session::completed_animations::bytes(
                        n["owner"].as_str().context("shade owner result absent")?
                    )? == header
                    && word(
                        &crate::original_session::completed_animations::bytes(
                            n["epoch"].as_str().context("shade result epoch absent")?
                        )?,
                        0
                    ) == threshold,
                "shade getter cache result differs"
            );
        }
        if matches!(pc, 0x49f42d | 0x49f628 | 0x49f949) {
            if let Some(raw) = row["sprite_clip_margins"].as_str() {
                let bytes = crate::original_session::completed_animations::bytes(raw)?;
                ensure!(
                    std::array::from_fn::<_, 4, _>(|n| word(&bytes, n * 4) as i32)
                        == self.margins.unwrap_or([0; 4]),
                    "sprite clip margins differ"
                );
            } else {
                ensure!(
                    self.margins.is_none(),
                    "clipped sprite observed margins absent"
                );
            }
            let frame = crate::original_session::completed_animations::bytes(
                row["sprite_frame"]["raw"]
                    .as_str()
                    .context("sprite locals absent")?,
            )?;
            let bp = row["sprite_frame"]["bp"].as_u64().unwrap() as u32;
            // Native addresses label the two observed planes. They are used
            // only to compare aliases, never to supply rendering state.
            let planes = row["buffers"]
                .as_array()
                .context("sprite observed planes absent")?;
            ensure!(planes.len() == 2, "sprite observed plane count differs");
            for (pointer, base, pitch) in [
                (
                    word(&stack, 4),
                    planes[0]["pointer"]
                        .as_u64()
                        .context("sprite observed color root absent")? as u32,
                    data.color_pitch,
                ),
                (
                    word(&stack, 8),
                    planes[1]["pointer"]
                        .as_u64()
                        .context("sprite observed depth root absent")? as u32,
                    data.depth_pitch,
                ),
            ] {
                let at = usize::try_from(i64::from(pointer) - (i64::from(bp) - 0x5c))
                    .context("sprite parameter alias outside frame")?;
                let offset = self.origin[1]
                    .wrapping_mul(pitch)
                    .wrapping_add(self.origin[0] * 2);
                ensure!(
                    word(&frame, at) == base.wrapping_add(offset as u32)
                        && word(&frame, at + 4) == (pitch / 2) as u32,
                    "sprite destination plane differs"
                );
            }
            ensure!(
                word(&stack, 12) == source.frame_address
                    && word(&stack, 16) == source.palette_address(self.palette_light)
                    && word(&stack, 20) == self.base.unwrap() as u32
                    && word(&stack, 24)
                        == source.rows_address.wrapping_add(
                            ((self.origin[1] - source.origin_y) as u32).wrapping_mul(4)
                        ),
                "sprite blit arguments differ"
            );
        }
        if pc == 0x4b0000 {
            let raw = crate::original_session::completed_animations::bytes(
                row["floor_frame"]["raw"]
                    .as_str()
                    .context("sprite return locals absent")?,
            )?;
            ensure!(
                std::array::from_fn::<_, 4, _>(|i| word(&raw, 0xa0 + i * 4) as i32)
                    == self.rect.unwrap(),
                "sprite returned bounds differ"
            );
        }
        Ok(())
    }
}
