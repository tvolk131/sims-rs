//!53e8f8 visible prelude. Shared placement/cache/light stores stay with their
//! retained owners; material lookup and raster drawing remain explicit children.
use super::*;
use sims_vm::scheduler::clock::draw::context::{
    placement::PlacementUpdate, Frame as DrawContext, LightUpdate,
};
type Row = crate::original_session::draw_parent::world::paint::row::State;
type RasterJob = crate::original_session::draw_parent::world::paint::row::GeometryRasterJob;
type RasterParams = crate::original_session::draw_parent::world::paint::row::GeometryRasterParams;
const MATERIAL_SPECIAL: &str = "c7d259decfb58e83a2c71f3851c4abfe52203f7518ec582a6c91d102b9cf94e8";
const MATERIAL_PLAIN: &str = "cc11ed5c053784f0c6c2c61b9496652a19ff10ffaa7654da530405dfa057bd5e";
fn zero(v: &u8) -> bool {
    *v == 0
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Rotate,
    Rotated,
    Center,
    Centered,
    Getter,
    GetterReturned,
    Place,
    Placing,
    Placed,
    Light,
    LightReturned,
    Material,
    MaterialReturned,
    Select,
    Selected,
    PrepareDraw,
    Flags,
    FlagsReturned,
    Descriptor,
    DescriptorReturned,
    Palette,
    PaletteReturned,
    Renderer,
    RendererReturned,
    ClipFirstRestored,
    Complete,
    Overlay,
    OverlayLight,
    OverlayLightReturned,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Materials {
    source: String,
    pool: u32,
    key: u32,
    images: [u32; 3],
    palette: u32,
    default_palette: u32,
}
impl Materials {
    fn import(encoded: &[u8]) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        let key = match source.as_str() {
            MATERIAL_SPECIAL => 250,
            MATERIAL_PLAIN => 6,
            _ => anyhow::bail!("visible material source differs"),
        };
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "visible material epoch differs"
        );
        let m = Memory::new(&v)?;
        let pool = word(&m.read(0x6f0bf8, 4)?, 0);
        let body = m.read(pool, 0x40)?;
        let head = word(&body, 0);
        let mut pending = vec![word(&m.read(head, 24)?, 4)];
        let mut seen = std::collections::BTreeSet::new();
        let mut matched = None;
        while let Some(p) = pending.pop() {
            if p == 0 || p == head {
                continue;
            }
            ensure!(
                seen.insert(p) && seen.len() <= 8192,
                "material directory cycle"
            );
            let node = m.read(p, 24)?;
            pending.extend([word(&node, 8), word(&node, 12)]);
            if word(&node, 16) == key {
                ensure!(matched.is_none(), "duplicate retained material");
                matched = Some((p, word(&node, 20)));
            }
        }
        let (node, payload) = matched.context("retained material absent")?;
        let raw = m.read(payload, 20)?;
        let index = word(&raw, 0) as usize;
        let begin = word(&body, 0x24);
        let end = word(&body, 0x28);
        ensure!(
            begin <= end && (end - begin) % 8 == 0 && index * 8 + 8 <= (end - begin) as usize,
            "material palette vector differs"
        );
        let palettes = m.read(begin, (end - begin) as usize)?;
        let palette = word(&palettes, index * 8 + 4);
        let default_palette = word(&palettes, 4);
        let images = std::array::from_fn(|i| word(&raw, 4 + 4 * i));
        ensure!(
            v["pool"] == pool
                && v["material"]["node"] == node
                && v["material"]["payload"] == payload
                && v["material"]["palette"] == palette
                && v["material"]["default_palette"] == default_palette
                && v["material"]["images"] == serde_json::to_value(images)?,
            "material B aliases differ"
        );
        Ok(Self {
            source,
            pool,
            key,
            images,
            palette,
            default_palette,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Passes {
    count: u32,
    index: u32,
    active: u8,
    edge: u8,
    outside_light: u32,
}
fn door(style: u8) -> bool {
    matches!(style, 3 | 5 | 6 | 15 | 23)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    point: [u8; 3],
    rotation: u8,
    height: u8,
    light: u32,
    record: [u8; 6],
    side: u8,
    zoom: u8,
    mask: u32,
    rectangle: [i32; 4],
    stage: Stage,
    rotated: Option<[u8; 3]>,
    physical: Option<[i32; 2]>,
    placement: Option<PlacementUpdate>,
    light_update: Option<LightUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    materials: Option<Materials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mask_image: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color_image: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selector_palette: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lighting: Option<[u8; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    descriptor_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raster: Option<RasterJob>,
    #[serde(default, skip_serializing_if = "zero")]
    active: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passes: Option<Passes>,
}
impl State {
    pub(super) fn input_request(&self) -> Option<super::super::super::super::super::GeometryInput> {
        if self.stage != Stage::Material || self.materials.is_some() { return None; }
        let at = 2 + 2 * usize::from(self.side);
        Some(super::super::super::super::super::GeometryInput::Materials {
            key: u32::from(u16::from_le_bytes(self.record[at..at+2].try_into().ok()?)),
        })
    }
    pub fn new(
        point: [u8; 3],
        rotation: u8,
        height: u8,
        light: u32,
        record: [u8; 6],
        side: u8,
        zoom: u8,
        mask: u32,
        rectangle: [i32; 4],
        active: u8,
    ) -> Self {
        Self {
            point,
            rotation,
            height,
            light,
            record,
            side,
            zoom,
            mask,
            rectangle,
            stage: Stage::Entry,
            rotated: None,
            physical: None,
            placement: None,
            light_update: None,
            materials: None,
            mask_image: None,
            color_image: None,
            selector_palette: None,
            lighting: None,
            descriptor_enabled: None,
            raster: None,
            active,
            passes: None,
        }
    }
    pub fn bind_materials(&mut self, encoded: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(
            self.stage == Stage::Material && self.materials.is_none(),
            "visible material binding escaped lookup"
        );
        self.materials = Some(Materials::import(encoded)?);
        self.validate()
    }
    fn begin_passes(
        &mut self,
        controls: Option<&sims_vm::interface::state::State>,
        outside_light: u32,
    ) -> Result<()> {
        if let Some(c) = controls {
            //53e968..53ea2b. These are reads of the actual current shared
            //CPState; captured getter answers are never installed here.
            let active = if level_geometry::special_kind(u32::from(self.record[1])) {
                0
            } else {
                self.active
            };
            let mut count = if self.record[0] & 14 != 0 { 2 } else { 1 };
            let allows = || c.mode != 0 || !matches!(c.build_tool, 8 | 9); //53d5f9,416920,4aaf34
            if active != 0 && door(self.record[1]) && !allows() {
                count = 0;
            }
            if count == 2 && active != 0 && c.mode != 0 {
                if self.record[1] == 22 && self.record[0] & 6 == 0 {
                    count = 1;
                } else if door(self.record[1]) {
                    count = 0;
                }
            }
            self.passes = Some(Passes {
                count,
                index: 0,
                active,
                edge: 0,
                outside_light,
            });
            if count == 0 {
                self.stage = Stage::Complete;
                return Ok(());
            }
        } else {
            ensure!(
                level_geometry::special_kind(u32::from(self.record[1])) || self.active == 0,
                "visible active material setup is pending"
            );
            ensure!(
                self.record[0] & 14 == 0,
                "visible additional material passes are pending"
            );
        }
        self.light_update = Some(LightUpdate::new(self.light));
        self.stage = Stage::Light;
        Ok(())
    }
    fn effective_active(&self) -> u8 {
        self.passes.as_ref().map_or_else(
            || {
                if level_geometry::special_kind(u32::from(self.record[1])) {
                    0
                } else {
                    self.active
                }
            },
            |p| p.active,
        )
    }
    fn selected_style(&self) -> u8 {
        if self.record[1] == 22 && self.effective_active() != 0 {
            1
        } else {
            self.record[1]
        }
    }
    fn overlay(&self) -> bool {
        self.passes.as_ref().is_some_and(|p| p.index == 1)
    }
    fn coverage_cache(&self) -> u32 {
        self.materials.as_ref().unwrap().default_palette
    }
    fn wallpaper_cache(&self) -> u32 {
        if self.overlay() {
            0
        } else {
            self.materials.as_ref().unwrap().palette
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.rotation < 4 && self.side < 2 && self.zoom <= 3 && self.active <= 1,
            "visible geometry rotation/side invalid"
        );
        ensure!(
            self.rotated.is_some() == !matches!(self.stage, Stage::Entry | Stage::Rotate),
            "visible rotated-point lifetime differs"
        );
        ensure!(
            self.physical.is_some()
                == !matches!(
                    self.stage,
                    Stage::Entry | Stage::Rotate | Stage::Rotated | Stage::Center
                ),
            "visible center lifetime differs"
        );
        if let Some(m) = &self.materials {
            ensure!(
                ((m.source == MATERIAL_SPECIAL && m.key == 250)
                    || (m.source == MATERIAL_PLAIN && m.key == 6)
                    || (crate::original_session::draw_parent::world::paint::row::is_catalog_source(
                        &m.source
                    ) && m.key <= u16::MAX as u32))
                    && m.pool != 0
                    && m.palette != 0
                    && m.default_palette != 0,
                "visible material owner differs"
            );
        }
        if let Some(p) = &self.passes {
            ensure!(
                p.count <= 2
                    && ((p.count == 0 && p.index == 0) || p.index < p.count)
                    && p.active <= 1
                    && matches!(p.edge, 0 | 7 | 8),
                "visible material loop state differs"
            );
        }
        let skipped = self.passes.as_ref().is_some_and(|p| p.count == 0);
        if let Some(l) = &self.light_update {
            l.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(
            self.mask_image.is_some()
                == (!skipped
                    && matches!(
                        self.stage,
                        Stage::MaterialReturned
                            | Stage::Select
                            | Stage::Selected
                            | Stage::PrepareDraw
                            | Stage::Flags
                            | Stage::FlagsReturned
                            | Stage::Descriptor
                            | Stage::DescriptorReturned
                            | Stage::Palette
                            | Stage::PaletteReturned
                            | Stage::Renderer
                            | Stage::RendererReturned
                            | Stage::ClipFirstRestored
                            | Stage::Complete
                            | Stage::OverlayLight
                            | Stage::OverlayLightReturned
                    )),
            "visible material result lifetime differs"
        );
        ensure!(
            self.color_image.is_some()
                == (!skipped
                    && matches!(
                        self.stage,
                        Stage::Selected
                            | Stage::PrepareDraw
                            | Stage::Flags
                            | Stage::FlagsReturned
                            | Stage::Descriptor
                            | Stage::DescriptorReturned
                            | Stage::Palette
                            | Stage::PaletteReturned
                            | Stage::Renderer
                            | Stage::RendererReturned
                            | Stage::ClipFirstRestored
                            | Stage::Complete
                            | Stage::OverlayLight
                            | Stage::OverlayLightReturned
                    )),
            "visible selected image lifetime differs"
        );
        if let Some(r) = &self.raster {
            r.validate()?;
            ensure!(
                matches!(
                    self.stage,
                    Stage::Renderer
                        | Stage::RendererReturned
                        | Stage::ClipFirstRestored
                        | Stage::Complete
                ),
                "visible renderer escaped caller"
            );
        }
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn advance(
        &mut self,
        sim: &sims_sim::Simulation,
        context: &mut DrawContext,
        row: Option<&mut Row>,
        clip: [i32; 4],
        controls: Option<&sims_vm::interface::state::State>,
        outside_light: u32,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Entry => self.stage = Stage::Rotate,
            Stage::Rotate => {
                let [x, y, z] = self.point;
                self.rotated = Some(match 4u8.wrapping_sub(self.rotation) & 3 {
                    1 => [63u8.wrapping_sub(y), x, z],
                    2 => [63u8.wrapping_sub(x), 63u8.wrapping_sub(y), z],
                    3 => [y, 63u8.wrapping_sub(x), z],
                    _ => [x, y, z],
                });
                self.stage = Stage::Rotated;
            }
            Stage::Rotated => self.stage = Stage::Center,
            Stage::Center => {
                let [x, y, _] = self.rotated.unwrap().map(|v| v as i8);
                self.physical = Some([(i32::from(y) << 4) | 8, (i32::from(x) << 4) | 8]);
                self.stage = Stage::Centered;
            }
            Stage::Centered => self.stage = Stage::Getter,
            Stage::Getter => self.stage = Stage::GetterReturned,
            Stage::GetterReturned => {
                self.placement = Some(PlacementUpdate::new(
                    self.physical.unwrap(),
                    i32::from(self.point[2] as i8),
                    i32::from(self.height),
                    self.rotation,
                ));
                self.stage = Stage::Place;
            }
            Stage::Place => self.stage = Stage::Placing,
            Stage::Placing => {
                let update = self.placement.as_mut().unwrap();
                let progress = row
                    .context("visible geometry lost its shared placement/cache owner")?
                    .advance_geometry_placement(update, sim, context)?;
                ensure!(
                    progress || update.complete(),
                    "visible placement child stalled"
                );
                if update.complete() {
                    self.stage = Stage::Placed;
                }
            }
            Stage::Placed => self.begin_passes(controls, outside_light)?,
            Stage::Light => {
                let update = self.light_update.as_mut().unwrap();
                context.advance_light(update).map_err(anyhow::Error::msg)?;
                if update.complete() {
                    self.stage = Stage::LightReturned;
                }
            }
            Stage::LightReturned => {
                self.stage = if self.overlay() {
                    Stage::Overlay
                } else {
                    Stage::Material
                }
            }
            Stage::Material => {
                if self.materials.is_none() {
                    let at = 2 + 2 * usize::from(self.side);
                    let key = u32::from(u16::from_le_bytes(self.record[at..at + 2].try_into()?));
                    if let Some(r) = row {
                        if let Some(m) = r.lookup_geometry_material(key)? {
                            self.materials = Some(Materials {
                                source: m.source,
                                pool: m.pool,
                                key: m.key,
                                images: m.images,
                                palette: m.palette,
                                default_palette: m.default_palette,
                            });
                        }
                    }
                }
                let Some(m) = &self.materials else {
                    return Ok(false);
                };
                let at = 2 + 2 * usize::from(self.side);
                let key = u16::from_le_bytes(self.record[at..at + 2].try_into()?);
                ensure!(
                    u32::from(key) == m.key && (1..=3).contains(&self.zoom),
                    "visible material entry/zoom is unobserved"
                );
                self.mask_image = Some(m.images[usize::from(self.zoom) - 1]);
                self.stage = Stage::MaterialReturned;
            }
            Stage::MaterialReturned => {
                if self.passes.is_some() && self.record[0] & 6 != 0 && !door(self.record[1]) {
                    let edge = if self.record[0] & 2 != 0 { 8 } else { 7 };
                    self.passes.as_mut().unwrap().edge = edge;
                    self.color_image = Some(
                        ((u32::from(self.zoom) << 8) + u32::from(edge))
                            .wrapping_mul(2)
                            .wrapping_sub(512),
                    );
                    self.stage = Stage::PrepareDraw;
                } else {
                    ensure!(
                        self.record[0] & 6 == 0 || door(self.record[1]),
                        "visible overridden material path is pending"
                    );
                    self.stage = Stage::Select;
                }
            }
            Stage::Select => {
                let style = self.selected_style();
                if matches!(style, 22 | 23) {
                    let portal = row.context("junction selector row absent")?.lookup_portal(
                        sim,
                        self.rotated.unwrap(),
                        self.rotation,
                        self.mask,
                    )?;
                    let (cache, picture) = portal.unwrap_or((0, 0));
                    if picture == 0 {
                        self.selector_palette = Some(0);
                        self.color_image = Some(
                            (u32::from(self.zoom) << 9)
                                .wrapping_sub(510)
                                .wrapping_add(u32::from(self.effective_active() != 0)),
                        );
                    } else {
                        self.selector_palette = Some(cache);
                        self.color_image = Some(
                            (i32::from(picture) as u32)
                                .wrapping_add(u32::from(self.effective_active()) * 3)
                                .wrapping_sub(1)
                                .wrapping_add(u32::from(self.zoom)),
                        );
                    }
                } else {
                    self.selector_palette = None;
                    self.color_image = Some(
                        (u32::from(self.zoom)
                            .wrapping_mul(256)
                            .wrapping_add(u32::from(style)))
                        .wrapping_mul(2)
                        .wrapping_sub(512)
                        .wrapping_add(u32::from(self.effective_active() != 0)),
                    );
                }
                self.stage = Stage::Selected;
            }
            Stage::Selected => self.stage = Stage::PrepareDraw,
            Stage::Overlay => {
                self.selector_palette = None;
                let p = self.passes.as_ref().context("overlay pass absent")?;
                ensure!(
                    p.edge != 0,
                    "visible ordinary second material pass is pending"
                );
                ensure!(
                    self.mask != 32,
                    "visible diagonal overlay rectangle adjustment is pending"
                );
                let first = self.mask == 32 || p.edge == 7;
                self.color_image =
                    Some((u32::from(self.zoom) << 9).wrapping_sub(if first { 494 } else { 492 }));
                self.mask_image =
                    Some((u32::from(self.zoom) << 8).wrapping_add(if first { 1532 } else { 1533 }));
                self.light_update = Some(LightUpdate::new(p.outside_light));
                self.stage = Stage::OverlayLight;
            }
            Stage::OverlayLight => {
                let update = self.light_update.as_mut().unwrap();
                context.advance_light(update).map_err(anyhow::Error::msg)?;
                if update.complete() {
                    self.stage = Stage::OverlayLightReturned;
                }
            }
            Stage::OverlayLightReturned => self.stage = Stage::PrepareDraw,
            Stage::PrepareDraw => {
                self.lighting = Some([127; 4]);
                self.stage = Stage::Flags;
            }
            Stage::Flags => {
                self.descriptor_enabled =
                    Some(context.state().flags(3).map_err(anyhow::Error::msg)? & 1 == 0);
                self.stage = Stage::FlagsReturned;
            }
            Stage::FlagsReturned => {
                self.stage = if self.descriptor_enabled == Some(true) && !self.overlay() {
                    Stage::Descriptor
                } else {
                    Stage::Palette
                }
            }
            Stage::Descriptor => {
                self.lighting = Some(
                    row.context("visible descriptor owner absent")?
                        .geometry_descriptor(self.point.map(|v| v as i8), self.rotation)?,
                );
                self.stage = Stage::DescriptorReturned;
            }
            Stage::DescriptorReturned => self.stage = Stage::Palette,
            Stage::Palette => self.stage = Stage::PaletteReturned,
            Stage::PaletteReturned => self.stage = Stage::Renderer,
            Stage::Renderer => {
                let row = row.context("visible renderer lost its shared owners")?;
                if let Some(job) = &mut self.raster {
                    row.advance_geometry_raster(job, context)?;
                    if job.complete() {
                        self.stage = Stage::RendererReturned;
                    }
                } else {
                    if !row.geometry_raster_ready() {
                        return Ok(false);
                    }
                    let m = self.materials.as_ref().unwrap();
                    self.raster = Some(RasterJob::new(RasterParams {
                        coverage_cache: self.coverage_cache(),
                        coverage_override: self.selector_palette.unwrap_or(0),
                        coverage_image: self.color_image.unwrap(),
                        wallpaper_cache: self.wallpaper_cache(),
                        wallpaper_image: self.mask_image.unwrap(),
                        index: match self.mask {
                            1 => 0,
                            2 => 1,
                            16 => 2,
                            32 => 3,
                            _ => anyhow::bail!("unknown visible geometry frame"),
                        },
                        point: [self.rectangle[0], self.rectangle[1]],
                        offset: if self.mask == 2 {
                            8i32.wrapping_shl(u32::from(self.zoom))
                        } else {
                            0
                        },
                        mask: self.mask,
                        clip,
                        descriptor: self.lighting.unwrap(),
                    })?);
                }
            }
            Stage::RendererReturned => {
                row.context("clip restore lost row")?
                    .request_geometry_clip(clip, 0)?;
                self.stage = Stage::ClipFirstRestored;
            }
            Stage::ClipFirstRestored => {
                row.context("clip restore lost row")?
                    .request_geometry_clip(clip, 1)?;
                if self.passes.as_ref().is_some_and(|p| p.index + 1 < p.count) {
                    self.passes.as_mut().unwrap().index += 1;
                    self.mask_image = None;
                    self.color_image = None;
                    self.lighting = None;
                    self.descriptor_enabled = None;
                    self.raster = None;
                    self.light_update = Some(LightUpdate::new(self.light));
                    self.stage = Stage::Light;
                } else {
                    self.stage = Stage::Complete;
                }
            }
            Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_cut(&self, pc: u32) -> bool {
        if self.stage == Stage::Renderer {
            if let Some(r) = &self.raster {
                return r.at_cut(pc);
            }
        }
        matches!(
            (pc, self.stage),
            (0x53ebb5, Stage::OverlayLight)
                | (0x53ebba, Stage::OverlayLightReturned)
                | (0x53eb1b, Stage::Overlay)
                | (0x53e8f8, Stage::Entry)
                | (0x53e934, Stage::Rotate)
                | (0x53e939, Stage::Rotated)
                | (0x53e953, Stage::Center)
                | (0x53e958, Stage::Centered)
                | (0x53e95c, Stage::Getter)
                | (0x53e961, Stage::GetterReturned)
                | (0x53e963, Stage::Place)
                | (0x53e968, Stage::Placed)
                | (0x53ea46, Stage::Light)
                | (0x53ea4b, Stage::LightReturned)
                | (0x53eaba, Stage::Material)
                | (0x53eabf, Stage::MaterialReturned)
                | (0x53eaf1, Stage::Select)
                | (0x53eaf6, Stage::Selected)
                | (0x53ebfb, Stage::PrepareDraw)
                | (0x53ecac, Stage::Flags)
                | (0x53ecb1, Stage::FlagsReturned)
                | (0x53ecdf, Stage::Descriptor)
                | (0x53ece4, Stage::DescriptorReturned)
                | (0x53ed40, Stage::Palette)
                | (0x53ed45, Stage::PaletteReturned)
                | (0x53ed47, Stage::Renderer)
                | (0x53ed4c, Stage::RendererReturned)
                | (0x53ed5d, Stage::ClipFirstRestored)
        )
    }
    pub fn compare(&self, q: &serde_json::Value, graphics: GeometryView<'_>) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("visible geometry PC absent")? as u32;
        ensure!(self.at_cut(pc), "visible geometry stage differs");
        if self.stage == Stage::Renderer {
            if let Some(r) = &self.raster {
                let (row, context) = graphics.context("renderer comparison lost shared owners")?;
                return row.compare_geometry_raster(r, q, context);
            }
        }
        let p = &q["geometry_prefix"];
        let f = decode(&p["frame"]["raw"])?;
        let stack = decode(&p["stack"]["raw"])?;
        let bp = q["child_frame"]["bp"]
            .as_u64()
            .context("visible child BP absent")? as u32;
        if let Some(loop_) = &self.passes {
            ensure!(
                word(&f, 0x178 - 0x30) == loop_.count
                    && word(&f, 0x178 - 0x40) == loop_.index
                    && f[0x178 - 0xd] == loop_.active,
                "visible native loop count/index/active differs"
            );
        }
        if self.rotated.is_some() {
            ensure!(
                f[0x178 - 0x8c..0x178 - 0x89] == self.rotated.unwrap(),
                "visible inverse point differs"
            );
        }
        if self.physical.is_some() {
            let x = self.physical.unwrap();
            ensure!(
                word(&f, 0x178 - 0x128) as i32 == x[0] && word(&f, 0x178 - 0x124) as i32 == x[1],
                "visible physical center differs"
            );
        }
        match self.stage {
            Stage::Rotate => ensure!(
                word(&stack, 0) == u32::from(4u8.wrapping_sub(self.rotation) & 3)
                    && q["registers"]["ecx"] == bp - 0x8c,
                "visible rotation ABI differs"
            ),
            Stage::Center => ensure!(
                word(&stack, 0) == bp - 0x128
                    && word(&stack, 4) == u32::from(self.point[2] as i8 as u8)
                    && word(&stack, 8) == q["context"]["pointer"].as_u64().unwrap() as u32
                    && word(&stack, 12) == u32::from(self.height),
                "visible center ABI differs"
            ),
            Stage::Getter | Stage::Place => ensure!(
                word(&stack, 0) == bp - 0x128,
                "visible placement point alias differs"
            ),
            Stage::GetterReturned => ensure!(
                q["registers"]["eax"] == q["context"]["pointer"],
                "visible context getter differs"
            ),
            Stage::OverlayLight => ensure!(
                word(&stack, 0) == self.passes.as_ref().unwrap().outside_light,
                "visible overlay light argument differs"
            ),
            Stage::Light => ensure!(
                word(&stack, 0) == self.light,
                "visible material light argument differs"
            ),
            Stage::Material => {
                let at = 2 + 2 * usize::from(self.side);
                ensure!(
                    word(&stack, 0)
                        == u32::from(u16::from_le_bytes(self.record[at..at + 2].try_into()?)),
                    "visible material pattern differs"
                );
            }
            Stage::MaterialReturned => ensure!(
                q["registers"]["eax"]
                    .as_u64()
                    .context("material AL absent")?
                    & 255
                    == 1
                    && word(&f, 0x178 - 0x50) == self.materials.as_ref().unwrap().palette
                    && word(&f, 0x178 - 0x104) == self.mask_image.unwrap(),
                "visible material result differs"
            ),
            Stage::Select => ensure!(
                word(&stack, 0) == u32::from(self.selected_style())
                    && word(&stack, 4) == bp - 0x74
                    && word(&stack, 8) == bp - 0x1c
                    && word(&stack, 16) == u32::from(self.zoom)
                    && word(&stack, 20) == u32::from(self.effective_active()),
                "visible image selector ABI differs"
            ),
            Stage::Selected => ensure!(
                q["registers"]["eax"] == self.color_image.unwrap()
                    && word(&f, 0x178 - 0x74) == self.selector_palette.unwrap_or(0),
                "visible image selector result differs"
            ),
            Stage::PrepareDraw => ensure!(
                q["registers"]["ebx"] == self.color_image.unwrap(),
                "visible selected image lost before drawing"
            ),
            Stage::Flags => ensure!(
                word(&stack, 0) == 3 && word(&stack, 4) == 1,
                "visible flag getter ABI differs"
            ),
            Stage::FlagsReturned => ensure!(
                q["registers"]["eax"].as_u64().context("flag AL absent")? & 255
                    == u64::from(self.descriptor_enabled.unwrap()),
                "visible flag getter result differs"
            ),
            Stage::DescriptorReturned => {
                let address = q["registers"]["eax"]
                    .as_u64()
                    .context("descriptor pointer absent")? as u32;
                let start = bp - 0x178;
                let at = usize::try_from(
                    address
                        .checked_sub(start)
                        .context("descriptor result outside child")?,
                )?;
                ensure!(
                    f.get(at..at + 4) == Some(self.lighting.unwrap().as_slice()),
                    "visible descriptor result differs"
                );
            }
            Stage::PaletteReturned => ensure!(
                q["registers"]["eax"] == self.coverage_cache(),
                "visible default palette getter differs"
            ),
            Stage::Renderer => {
                let m = self.materials.as_ref().unwrap();
                let frame = match self.mask {
                    1 => 0,
                    2 => 1,
                    16 => 2,
                    32 => 3,
                    _ => anyhow::bail!("visible frame mask unsupported"),
                };
                let expected = [
                    bp - 0x170,
                    self.selector_palette.unwrap_or(0),
                    self.color_image.unwrap(),
                    self.wallpaper_cache(),
                    self.mask_image.unwrap(),
                    frame,
                    self.rectangle[0] as u32,
                    self.rectangle[1] as u32,
                    0,
                    q["context"]["pointer"].as_u64().unwrap() as u32,
                    if self.mask == 2 {
                        8u32.wrapping_shl(u32::from(self.zoom))
                    } else {
                        0
                    },
                    self.mask,
                    bp - 0x54,
                    0,
                ];
                ensure!(
                    q["registers"]["ecx"] == self.coverage_cache()
                        && expected
                            .iter()
                            .enumerate()
                            .all(|(i, v)| word(&stack, 4 * i) == *v),
                    "visible renderer receiver/arguments differ"
                );
                ensure!(
                    f[0x178 - 0x54..0x178 - 0x50] == self.lighting.unwrap(),
                    "visible renderer descriptor argument differs"
                );
            }
            Stage::RendererReturned => {
                let r = self.raster.as_ref().context("returned renderer absent")?;
                ensure!(
                    r.complete()
                        && std::array::from_fn::<_, 4, _>(|i| word(&f, 8 + 4 * i) as i32)
                            == r.rectangle()?,
                    "C4 returned rectangle differs"
                );
            }
            _ => (),
        }
        Ok(())
    }
}
