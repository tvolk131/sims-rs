//!53e1f7 side prefix and4c8849 C3 child; all arguments come from the caller.
use super::*;
use sims_sim::lot_presenter::{
    dual_sprite as raster, room_masks::Camera, sprite_full, terrain_raster::Data,
};
const INPUT: &str = "247adc25ae93303d450ac27422d071f4a814a8d7f856d28e9ecc89d7d258eba5";
const MATERIAL: &str = "edb99ccf9965bb171ad8c1040481cd42335bd3e1ca699c4f6747b785f84b91ba";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    targets: sims_vm::scheduler::clock::draw::targets::Buffers,
    source: String,
    cache: u32,
    resource: u32,
    raw: Vec<u8>,
    frame: u32,
    encoded: Vec<u8>,
    palette: Vec<u8>,
    palette_header: Vec<u8>,
    depth: Vec<u8>,
    rows: Vec<i32>,
    heights: Vec<i16>,
    lookup: Vec<u8>,
    project: [u32; 4],
    mask: Vec<u8>,
    indices: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frames: Option<Frames>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Frames {
    source: String,
    items: std::collections::BTreeMap<u32, (u32, Vec<u8>)>,
}
fn zero(v: &u8) -> bool {
    *v == 0
}
fn short(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
impl Source {
    pub(super) fn cache(&self) -> u32 { self.cache }
    pub(super) fn corner_vertical_offset(&self, zoom: u8) -> Result<i32> {
        self.validate()?;
        ensure!((1..=3).contains(&zoom), "corner side depth zoom is pending");
        Ok(word(&self.depth, usize::from(30-zoom)*4) as i32)
    }
    pub fn import(
        input: &[u8],
        material: &[u8],
        targets: sims_vm::scheduler::clock::draw::targets::Buffers,
    ) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == INPUT && sims_fixtures::digest(material) == MATERIAL,
            "side B input differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        let t: serde_json::Value = serde_json::from_slice(material)?;
        ensure!(
            v["tick"] == 89073
                && t["tick"] == 89073
                && v["executes_target_code"] == false
                && t["executes_target_code"] == false,
            "side B epoch differs"
        );
        let m = Memory::new(&v)?;
        let tm = Memory::new(&t)?;
        let r = &t["resources"][0];
        ensure!(
            r["image"] == 4096 && r["resident"] == true,
            "side sprite was not resident in B"
        );
        let cache = r["palette"].as_u64().context("side cache absent")? as u32;
        let resource = r["resource"].as_u64().context("side resource absent")? as u32;
        let raw = tm.read(resource, 44)?;
        ensure!(
            m.read(resource, 44)? == raw,
            "side resource B aliases differ"
        );
        let c = tm.read(cache, 128)?;
        let (a, z) = (word(&c, 8), word(&c, 12));
        ensure!(a < z && (z - a) % 4 == 0, "side cache directory differs");
        let mut p = word(&tm.read(a + 4096 % ((z - a) / 4) * 4, 4)?, 0);
        let mut found = 0;
        let mut seen = std::collections::BTreeSet::new();
        while p != 0 {
            ensure!(seen.insert(p) && seen.len() < 2048, "side cache cycle");
            let n = tm.read(p, 12)?;
            if word(&n, 4) == 4096 {
                found = word(&n, 8);
                break;
            }
            p = word(&n, 0);
        }
        ensure!(found == resource, "side B cache entry differs");
        let frame = word(&tm.read(word(&raw, 12), 8)?, 4);
        let prefix = tm.read(frame, 8192)?;
        let size = raster::frame_bytes(&prefix).map_err(anyhow::Error::msg)?;
        let encoded = prefix[..size].to_vec();
        let owner = word(&raw, 8);
        let palette = m.read(owner + 0x420, 512)?;
        let depth = m.read(0x9e88960 + 0xb8, 0xb0)?;
        let zoom = word(&m.read(0x6f00d0, 16)?, 8);
        ensure!(zoom == 1, "side zoom needs another resource frame");
        let rows = m
            .read(word(&depth, (7 - zoom) as usize * 4) - 512, 8192)?
            .chunks_exact(4)
            .map(|r| i32::from_le_bytes(r.try_into().unwrap()))
            .collect();
        let heights = m
            .read(word(&depth, (21 - zoom) as usize * 4), 512)?
            .chunks_exact(2)
            .map(|r| i16::from_le_bytes(r.try_into().unwrap()))
            .collect();
        let project = m.read(0x6f00d0, 16)?;
        let s = Self {
            targets,
            source: INPUT.into(),
            cache,
            resource,
            raw,
            frame,
            encoded,
            palette,
            palette_header: m.read(owner, 32)?,
            depth,
            rows,
            heights,
            lookup: m.read(0x6cd468, 256)?,
            project: std::array::from_fn(|i| word(&project, i * 4)),
            mask: m.read(0x6c6740, 32)?,
            indices: m.read(0x6efc2c, 32)?,
            frames: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(f) = &self.frames {
            ensure!(
                super::geometry_catalog::is_source(&f.source)
                    && f.items
                        .get(&1)
                        .is_some_and(|(p, b)| *p == self.frame && *b == self.encoded),
                "side frame catalog identity differs"
            );
        }
        ensure!(
            self.source == INPUT
                && self.raw.len() == 44
                && self.palette.len() == 512
                && self.palette_header.len() == 32
                && self.depth.len() == 0xb0
                && self.rows.len() == 2048
                && self.heights.len() == 256
                && self.lookup.len() == 256
                && self.mask.len() == 32
                && self.indices.len() == 32,
            "side retained owner differs"
        );
        Ok(())
    }
    pub(super) fn extend_catalog(&mut self, c: &super::geometry_catalog::Catalog) -> Result<()> {
        self.validate()?;
        c.validate()?;

        let r = c
            .resources
            .iter()
            .find(|r| r.cache == self.cache && r.image == 4096)
            .context("side catalog resource absent")?;
        ensure!(
            r.pointer == self.resource
                && r.header == self.raw
                && r.frames
                    .get(&1)
                    .is_some_and(|(p, b)| *p == self.frame && *b == self.encoded),
            "side catalog B frame aliases differ"
        );
        if let Some(old) = &self.frames {
            ensure!(
                old.items == r.frames,
                "side enrichment changes retained frames"
            );
            return Ok(());
        }
        self.frames = Some(Frames {
            source: c.source().into(),
            items: r.frames.clone(),
        });
        self.validate()
    }
    fn frame(&self, index: u32) -> Result<(u32, &[u8])> {
        if index == 1 {
            return Ok((self.frame, &self.encoded));
        }
        self.frames
            .as_ref()
            .and_then(|f| f.items.get(&index))
            .map(|(p, b)| (*p, b.as_slice()))
            .context("side selected frame has no retained B owner")
    }
    fn row(&self, index: i32) -> Result<i32> {
        self.rows
            .get(usize::try_from(
                index.checked_add(128).context("side row overflow")?,
            )?)
            .copied()
            .context("side row outside B span")
    }
    pub fn compare(&self, q: &serde_json::Value) -> Result<()> {
        let geometry = if q.get("corner_geometry").is_some() { &q["corner_geometry"] } else { &q["visible_geometry"] };
        if let Some(s) = geometry.get("scratch").filter(|s| {
            s.get("indices").is_some() && q["visible_geometry"].get("side_alt_frame").is_none()
        }) {
            ensure!(
                decode(&s["mask"])? == self.mask && decode(&s["indices"])? == self.indices,
                "side scratch owner differs"
            );
        }
        Ok(())
    }
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(v.as_str().context("side bytes absent")?)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub point: [u8; 3],
    pub rotation: u8,
    pub zoom: u8,
    pub height: u8,
    pub flags: u8,
    pub screen: [i32; 2],
    pub light: u32,
    pub clip: [i32; 4],
    #[serde(default, skip_serializing_if = "zero")]
    pub alternate: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
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
    Palette,
    PaletteReturned,
    Renderer,
    ResourceOne,
    ResourceOneReturned,
    ResourceTwo,
    ResourceTwoReturned,
    FrameOne,
    FrameOneReturned,
    FrameTwo,
    FrameTwoReturned,
    Setup,
    SelectPalette,
    SelectedPalette,
    SetupReturned,
    Clip,
    Clipped,
    Rect,
    RectReturned,
    PointDepth,
    PointDepthReturned,
    Depth,
    ColorEnabled,
    DepthEnabled,
    Blit,
    Drawing,
    BlitReturned,
    Returned,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    params: Params,
    stage: Stage,
    rotated: Option<[u8; 3]>,
    physical: Option<[i32; 2]>,
    placement: Option<PlacementUpdate>,
    light: Option<LightUpdate>,
    screen: Option<[i32; 2]>,
    rect: Option<[i32; 4]>,
    margins: [i32; 4],
    clipped: bool,
    base: Option<i32>,
    plan: Option<raster::ConstantPlan>,
    next: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alternate: Option<super::geometry_side_alt::Job>,
    /// C3 can be called directly by a corner, after its caller has already
    /// placed/lit the context. Never run the wall-side helper prelude again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    direct: Option<Direct>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Direct { point_owner: u32 }
impl Job {
    pub fn new(params: Params) -> Result<Self> {
        ensure!(
            ((params.alternate == 0 && matches!(params.flags, 1 | 2 | 4 | 5 | 8 | 10))
                || (params.alternate == 1 && params.flags == 10))
                && params.zoom == 1
                && params.rotation < 4,
            "side alternate edge/zoom is pending"
        );
        Ok(Self::initial(params))
    }
    fn initial(params: Params) -> Self {
        Self {
            params,
            stage: Stage::Rotate,
            rotated: None,
            physical: None,
            placement: None,
            light: None,
            screen: None,
            rect: None,
            margins: [0; 4],
            clipped: false,
            base: None,
            plan: None,
            next: 0,
            alternate: None,
            direct: None,
        }
    }
    pub fn new_corner(params: Params, point_owner: u32) -> Result<Self> {
        ensure!(params.alternate==0 && params.flags!=0 && params.zoom==1
            && params.rotation<4 && point_owner!=0,
            "direct corner C3 parameters are pending");
        let screen=params.screen;
        let mut job=Self::initial(params);
        job.stage=Stage::Renderer;
        job.screen=Some(screen);
        job.direct=Some(Direct { point_owner });
        job.validate()?;
        Ok(job)
    }
    fn index(&self, s: &Source) -> u32 {
        u32::from(s.lookup[self.params.flags as usize]).wrapping_sub(1)
    }
    fn frame<'a>(&self, s: &'a Source) -> Result<(u32, &'a [u8])> {
        s.frame(self.index(s))
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(d)=&self.direct {
            ensure!(d.point_owner!=0 && self.params.alternate==0 && self.params.flags!=0
                && self.params.rotation<4 && self.screen==Some(self.params.screen)
                && self.rotated.is_none() && self.physical.is_none()
                && self.placement.is_none() && self.light.is_none()
                && self.alternate.is_none() && self.stage as u8>=Stage::Renderer as u8,
                "direct corner C3 escaped its caller");
        }
        if let Some(a) = &self.alternate {
            a.validate()?;
            ensure!(
                self.params.alternate == 1
                    && matches!(
                        self.stage,
                        Stage::Renderer | Stage::Returned | Stage::Complete
                    ),
                "alternate side escaped caller"
            );
        }
        ensure!(
            ((self.params.alternate == 0 && (self.direct.is_some() || matches!(self.params.flags, 1 | 2 | 4 | 5 | 8 | 10)))
                || (self.params.alternate == 1 && self.params.flags == 10))
                && self.params.zoom == 1
                && self.next <= self.plan.as_ref().map_or(0, |p| p.writes.len()),
            "side continuation differs"
        );
        Ok(())
    }
    pub fn at_cut(&self, pc: u32) -> bool {
        if self.direct.is_some() && self.stage==Stage::Returned { return pc==0x53c6b7; }
        if self.params.alternate != 0 {
            if self.stage == Stage::Renderer {
                return self.alternate.as_ref().is_some_and(|s| s.at_cut(pc));
            }
            if self.stage == Stage::Palette {
                return pc == 0x53e35b;
            }
            if self.stage == Stage::PaletteReturned {
                return matches!(pc, 0x53e360 | 0x53e362);
            }
        }
        matches!(
            (pc, self.stage),
            (0x53e1f7, Stage::Rotate)
                | (0x53e1fc, Stage::Rotated)
                | (0x53e23e, Stage::Center)
                | (0x53e243, Stage::Centered)
                | (0x53e246, Stage::Getter)
                | (0x53e24b, Stage::GetterReturned)
                | (0x53e24d, Stage::Place)
                | (0x53e2f5, Stage::Light)
                | (0x53e2fa, Stage::LightReturned)
                | (0x53e3ad, Stage::Palette)
                | (0x53e3b4, Stage::PaletteReturned)
                | (0x4c8849, Stage::Renderer)
                | (0x4c8e0d, Stage::ResourceOne)
                | (0x4c8878, Stage::ResourceOneReturned)
                | (0x4c8e0d, Stage::ResourceTwo)
                | (0x4c8884, Stage::ResourceTwoReturned)
                | (0x4c19b1, Stage::FrameOne)
                | (0x4c88a0, Stage::FrameOneReturned)
                | (0x4c19b1, Stage::FrameTwo)
                | (0x4c88ac, Stage::FrameTwoReturned)
                | (0x4c7fdb, Stage::Setup)
                | (0x4c76d1, Stage::Setup)
                | (0x4b954a, Stage::SelectPalette)
                | (0x4c77a1, Stage::SelectedPalette)
                | (0x4c88d4, Stage::SetupReturned)
                | (0x4c77bc, Stage::Clip)
                | (0x4c8910, Stage::Clipped)
                | (0x4c89f9, Stage::RectReturned)
                | (0x4c9e38, Stage::PointDepth)
                | (0x4c8a79, Stage::PointDepthReturned)
                | (0x4c8aab, Stage::ColorEnabled)
                | (0x4b444f, Stage::ColorEnabled)
                | (0x4b4456, Stage::DepthEnabled)
                | (0x49f2a3, Stage::Blit)
                | (0x4c8afa, Stage::BlitReturned)
                | (0x53e3b9, Stage::Returned)
        )
    }
    pub(super) fn advance(
        &mut self,
        s: &mut Source,
        d: &mut Data,
        sim: &sims_sim::Simulation,
        c: &mut Frame,
        camera: &Camera,
        placement: &mut super::placement::Source,
        alt_source: Option<&mut super::geometry_side_alt::Source>,
    ) -> Result<bool> {
        self.validate()?;
        s.validate()?;
        self.stage = match self.stage {
            Stage::Rotate => {
                let [x, y, z] = self.params.point;
                self.rotated = Some(match 4u8.wrapping_sub(self.params.rotation) & 3 {
                    1 => [63u8.wrapping_sub(y), x, z],
                    2 => [63u8.wrapping_sub(x), 63u8.wrapping_sub(y), z],
                    3 => [y, 63u8.wrapping_sub(x), z],
                    _ => [x, y, z],
                });
                Stage::Rotated
            }
            Stage::Rotated => {
                let z = u32::from(self.params.zoom);
                self.screen = Some([
                    self.params.screen[0].wrapping_sub(1 + 8i32.wrapping_shl(z)),
                    self.params.screen[1]
                        .wrapping_sub(i32::from(self.params.height).wrapping_shl(z + 1))
                        .wrapping_sub(4i32.wrapping_shl(z))
                        .wrapping_sub(word(
                            &s.depth,
                            ((if self.params.alternate == 0 { 30 } else { 33 }) - z) as usize * 4,
                        ) as i32),
                ]);
                Stage::Center
            }
            Stage::Center => {
                let [x, y, _] = self.rotated.unwrap().map(|v| v as i8);
                self.physical = Some([(i32::from(y) << 4) | 8, (i32::from(x) << 4) | 8]);
                Stage::Centered
            }
            Stage::Centered => Stage::Getter,
            Stage::Getter => Stage::GetterReturned,
            Stage::GetterReturned => {
                self.placement = Some(PlacementUpdate::new(
                    self.physical.unwrap(),
                    i32::from(self.params.point[2] as i8),
                    -1,
                    self.params.rotation,
                ));
                Stage::Place
            }
            Stage::Place => Stage::Placing,
            Stage::Placing => {
                let p = self.placement.as_mut().unwrap();
                placement.advance(p, sim, c)?;
                if p.complete() {
                    self.light = Some(LightUpdate::new(self.params.light));
                    Stage::Light
                } else {
                    Stage::Placing
                }
            }
            Stage::Light => {
                let l = self.light.as_mut().unwrap();
                c.advance_light(l).map_err(anyhow::Error::msg)?;
                if l.complete() {
                    Stage::LightReturned
                } else {
                    Stage::Light
                }
            }
            Stage::LightReturned => Stage::Palette,
            Stage::Palette => Stage::PaletteReturned,
            Stage::PaletteReturned => {
                if self.params.alternate != 0 {
                    self.alternate = Some(super::geometry_side_alt::Job::new(
                        super::geometry_side_alt::Params {
                            cache: s.cache,
                            color_image: 5119 + u32::from(self.params.zoom),
                            coverage_image: 6143 + u32::from(self.params.zoom),
                            index: u32::from(s.lookup[self.params.flags as usize]).wrapping_sub(1),
                            screen: self.screen.unwrap(),
                            clip: self.params.clip,
                        },
                    )?);
                }
                Stage::Renderer
            }
            Stage::Renderer => {
                if let Some(alt) = &mut self.alternate {
                    alt.advance(
                        alt_source.context("alternate side resource owner absent")?,
                        d,
                        c,
                    )?;
                    if alt.complete() {
                        self.rect = Some(alt.rectangle()?);
                        Stage::Returned
                    } else {
                        Stage::Renderer
                    }
                } else {
                    Stage::ResourceOne
                }
            }
            Stage::ResourceOne => Stage::ResourceOneReturned,
            Stage::ResourceOneReturned => Stage::ResourceTwo,
            Stage::ResourceTwo => Stage::ResourceTwoReturned,
            Stage::ResourceTwoReturned => Stage::FrameOne,
            Stage::FrameOne => Stage::FrameOneReturned,
            Stage::FrameOneReturned => Stage::FrameTwo,
            Stage::FrameTwo => Stage::FrameTwoReturned,
            Stage::FrameTwoReturned => Stage::Setup,
            Stage::Setup => {
                let [x, y] = self.screen.unwrap();
                let (_, b) = self.frame(s)?;
                self.rect = Some([
                    i32::from(short(b, 2).wrapping_add(x as i16)),
                    i32::from(short(b, 0).wrapping_add(y as i16)),
                    i32::from(short(b, 6).wrapping_add(x as i16)),
                    i32::from(short(b, 4).wrapping_add(y as i16)),
                ]);
                Stage::SelectPalette
            }
            Stage::SelectPalette => {
                ensure!(
                    c.state().words()[18] == 1f32.to_bits(),
                    "side modified palette is pending"
                );
                Stage::SelectedPalette
            }
            Stage::SelectedPalette => Stage::SetupReturned,
            Stage::SetupReturned => Stage::Clip,
            Stage::Clip => {
                let r = self.rect.unwrap();
                let k = self.params.clip;
                self.clipped = !(r[0] < k[2] && r[1] < k[3] && k[0] < r[2] && k[1] < r[3]);
                if !self.clipped {
                    self.margins = [
                        (k[0] - r[0]).max(0),
                        (k[1] - r[1]).max(0),
                        (r[2] - k[2]).max(0),
                        (r[3] - k[3]).max(0),
                    ];
                }
                Stage::Clipped
            }
            Stage::Clipped => {
                if self.clipped {
                    Stage::Returned
                } else {
                    Stage::Rect
                }
            }
            Stage::Rect => Stage::RectReturned,
            Stage::RectReturned => {
                ensure!(
                    c.state().flags(3).map_err(anyhow::Error::msg)? & 1 == 0,
                    "side alternative depth getter is pending"
                );
                Stage::PointDepth
            }
            Stage::PointDepth => {
                let p = self.params.point.map(|v| v as i8);
                let z = u32::from(self.params.zoom);
                let index = (i32::from(p[0]) + i32::from(p[1])).wrapping_mul(4i32.wrapping_shl(z));
                let lot = sim
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("side terrain owner absent")?;
                let h = sims_sim::lot_presenter::room_masks::terrain_corners(
                    lot.size,
                    &lot.heights,
                    p,
                    self.params.rotation,
                )
                .map_err(anyhow::Error::msg)?;
                self.base = Some(
                    s.row(index)?
                        .wrapping_add(i32::from(s.heights[h[1] as usize])),
                );
                Stage::PointDepthReturned
            }
            Stage::PointDepthReturned => Stage::Depth,
            Stage::Depth => {
                let w = c.state().words();
                let z = u32::from(self.params.zoom);
                //4c8a8d reads the table anchored at -projection.y. The
                // destination top only indexes per-row depth in49f2a3;
                // quantization makes their constant corrections differ.
                let at = (s.project[1] as i32).wrapping_neg();
                let correction = s
                    .row(at)?
                    .wrapping_sub(s.row(at + 4i32.wrapping_shl(z) + 1)?);
                self.base = Some(
                    self.base
                        .unwrap()
                        .wrapping_add(
                            (word(&s.depth, (36 - z) as usize * 4) as i32)
                                .wrapping_add(65)
                                .wrapping_mul(w[3] as i32),
                        )
                        .wrapping_add(correction)
                        .min(65535),
                );
                Stage::ColorEnabled
            }
            Stage::ColorEnabled => {
                ensure!(
                    c.state().flags(0).map_err(anyhow::Error::msg)? & 1 != 0,
                    "side disabled color is pending"
                );
                Stage::DepthEnabled
            }
            Stage::DepthEnabled => {
                ensure!(
                    c.state().flags(0).map_err(anyhow::Error::msg)? & 2 != 0,
                    "side disabled depth is pending"
                );
                let r = self.rect.unwrap();
                let h = usize::try_from(r[3] - r[1])?;
                let at = r[1].wrapping_sub(s.project[1] as i32);
                let first = s.row(at)?;
                let rows = (0..h)
                    .map(|y| {
                        Ok(s.row(at + y as i32)?
                            .wrapping_sub(first)
                            .wrapping_add(self.base.unwrap()))
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.plan = Some(
                    raster::plan_constant(raster::ConstantInput {
                        frame: self.frame(s)?.1,
                        palette: &s.palette,
                        screen: [r[0], r[1]],
                        margins: self.margins,
                        color_pitch: d.color_pitch / 2,
                        depth_pitch: d.depth_pitch / 2,
                        rows: &rows,
                        mask_scratch: &s.mask,
                        index_scratch: &s.indices,
                        depth: &d.depth,
                    })
                    .map_err(anyhow::Error::msg)?,
                );
                Stage::Blit
            }
            Stage::Blit => Stage::Drawing,
            Stage::Drawing => {
                let p = self.plan.as_ref().unwrap();
                if let Some(w) = p.writes.get(self.next) {
                    sprite_full::apply(w, &mut d.color, &mut d.depth)
                        .map_err(anyhow::Error::msg)?;
                    self.next += 1;
                    Stage::Drawing
                } else {
                    s.mask.clone_from(&p.mask_scratch);
                    s.indices.clone_from(&p.index_scratch);
                    Stage::BlitReturned
                }
            }
            Stage::BlitReturned => Stage::Returned,
            Stage::Returned => Stage::Complete,
            Stage::Complete => return Ok(false),
            Stage::Placed => unreachable!(),
        };
        let _ = camera;
        self.validate()?;
        Ok(true)
    }
    pub(super) fn compare(
        &self,
        q: &serde_json::Value,
        s: &Source,
        alt: Option<&super::geometry_side_alt::Source>,
        c: &Frame,
    ) -> Result<()> {
        if self.stage == Stage::Renderer {
            if let Some(job) = &self.alternate {
                return job.compare(q, alt.context("alternate side comparison owner absent")?, c);
            }
        }
        let frame_pointer = if self.params.alternate == 0 {
            self.frame(s)?.0
        } else {
            s.frame
        };
        let pc = q["pc"].as_u64().context("side PC absent")? as u32;
        ensure!(self.at_cut(pc), "side stage differs");
        let e = if self.direct.is_some() { &q["corner_geometry"] } else { &q["visible_geometry"] };
        let stack = decode(
            e.get("stack")
                .and_then(|s| s.get("raw"))
                .unwrap_or(if self.direct.is_some() { &q["corner_prefix"]["stack"]["raw"] } else { &q["geometry_prefix"]["stack"]["raw"] }),
        )?;
        let caller = if self.direct.is_some() { &q["corner_prefix"] } else { &q["child_frame"] };
        let frame = decode(if self.direct.is_some() { &caller["frame"]["raw"] } else { &caller["raw"] })?;
        let bp = caller["bp"]
            .as_u64()
            .context("side parent BP absent")? as u32;
        if self.rotated.is_some() {
            ensure!(
                frame[0x178 - 0x98..0x178 - 0x95] == self.rotated.unwrap(),
                "side rotated point differs"
            );
        }
        if let Some(p) = self.physical {
            ensure!(
                word(&frame, 0x178 - 0x120) as i32 == p[0]
                    && word(&frame, 0x178 - 0x11c) as i32 == p[1],
                "side physical point differs"
            );
        }
        match self.stage {
            Stage::Rotate => ensure!(
                word(&stack, 0) == u32::from(4u8.wrapping_sub(self.params.rotation) & 3),
                "side rotation argument differs"
            ),
            Stage::Center => ensure!(
                word(&stack, 0) == bp - 0x120 && word(&stack, 12) == u32::MAX,
                "side physical center ABI differs"
            ),
            Stage::Light => ensure!(
                word(&stack, 0) == self.params.light,
                "side light argument differs"
            ),
            Stage::PaletteReturned | Stage::Renderer => {
                let offset = usize::from(self.stage == Stage::Renderer) * 4;
                let [x, y] = self.screen.unwrap();
                let expected = if let Some(d)=&self.direct {
                    ensure!(word(&stack,0)==0x53c6b7,"corner C3 return address differs");
                    vec![bp-0x78,d.point_owner,4095+u32::from(self.params.zoom),self.index(s),
                        x as u32,y as u32,0,q["context"]["pointer"].as_u64().unwrap()as u32,0,0]
                } else if self.params.alternate != 0 {
                    vec![
                        bp - 0x140,
                        5119 + u32::from(self.params.zoom),
                        6143 + u32::from(self.params.zoom),
                        u32::from(s.lookup[self.params.flags as usize]).wrapping_sub(1),
                        x as u32,
                        y as u32,
                        0,
                        q["context"]["pointer"].as_u64().unwrap() as u32,
                        0,
                    ]
                } else {
                    vec![
                        bp - 0x160,
                        bp - 0x1c,
                        4095 + u32::from(self.params.zoom),
                        u32::from(s.lookup[self.params.flags as usize]).wrapping_sub(1),
                        x as u32,
                        y as u32,
                        0,
                        q["context"]["pointer"].as_u64().unwrap() as u32,
                        0,
                        0,
                    ]
                };
                ensure!(
                    expected
                        .iter()
                        .enumerate()
                        .all(|(i, v)| word(&stack, offset + i * 4) == *v),
                    "computed side renderer arguments differ"
                );
            }
            Stage::ResourceOne | Stage::ResourceTwo => ensure!(
                q["registers"]["ecx"] == s.cache && word(&stack, 4) == 4096,
                "side resource query differs"
            ),
            Stage::ResourceOneReturned | Stage::ResourceTwoReturned => ensure!(
                q["registers"]["eax"] == s.resource,
                "side resource return differs"
            ),
            Stage::FrameOne | Stage::FrameTwo => ensure!(
                q["registers"]["ecx"] == s.resource && word(&stack, 4) == self.index(s),
                "side frame query differs"
            ),
            Stage::FrameOneReturned | Stage::FrameTwoReturned => ensure!(
                q["registers"]["eax"] == frame_pointer,
                "side frame result differs"
            ),
            Stage::SelectPalette => {
                ensure!(
                    q["registers"]["ecx"] == word(&s.raw, 8) && word(&stack, 4) == 1f32.to_bits(),
                    "side palette call differs"
                );
                let p = &e["palette_selection"];
                ensure!(
                    p["owner"] == word(&s.raw, 8)
                        && decode(&p["raw"])? == s.palette_header
                        && decode(&p["unit_palette"])? == s.palette,
                    "side retained palette bytes differ"
                );
            }
            Stage::Setup => {
                let native_bp = e["side_frame"]["bp"]
                    .as_u64()
                    .context("side renderer BP absent")? as u32;
                let context = q["context"]["pointer"].as_u64().unwrap() as u32;
                let [x, y] = self.screen.unwrap();
                let expected = if pc == 0x4c7fdb {
                    vec![
                        frame_pointer + 10,
                        frame_pointer + 10,
                        word(&s.raw, 8),
                        word(&s.raw, 32),
                        frame_pointer,
                        frame_pointer,
                        x as u32,
                        y as u32,
                        context,
                        0,
                        0,
                    ]
                } else {
                    vec![
                        frame_pointer + 10,
                        frame_pointer + 10,
                        word(&s.raw, 8),
                        word(&s.raw, 32),
                        frame_pointer,
                        x as u32,
                        y as u32,
                        context,
                        0,
                        1,
                    ]
                };
                ensure!(
                    q["registers"]["ecx"] == native_bp - 0x78
                        && expected
                            .iter()
                            .enumerate()
                            .all(|(i, v)| if pc == 0x4c76d1 && i == 8 {
                                word(&stack, 4 + i * 4) & 65535 == *v
                            } else {
                                word(&stack, 4 + i * 4) == *v
                            }),
                    "side constructor arguments differ"
                );
            }
            Stage::Clipped => ensure!(
                q["registers"]["eax"]
                    .as_u64()
                    .context("side clip result absent")?
                    & 255
                    == u64::from(self.clipped),
                "side clip result differs"
            ),
            Stage::PointDepth => ensure!(
                q["registers"]["ecx"] == 0x9e88960u32 + 0xb8
                    && word(&stack, 4) == self.direct.as_ref().map_or(bp-0x1c,|d|d.point_owner)
                    && word(&stack, 8) == u32::from(self.params.zoom)
                    && word(&stack, 12) == 0,
                "side point-depth arguments differ"
            ),
            Stage::SelectedPalette => ensure!(
                q["registers"]["eax"] == word(&s.raw, 8) + 0x420,
                "side palette return differs"
            ),
            Stage::PointDepthReturned => ensure!(
                q["registers"]["eax"] == self.base.unwrap() as u32,
                "side point depth result differs"
            ),
            Stage::ColorEnabled => {
                if pc == 0x4c8aab {
                    let f = decode(&e["side_frame"]["raw"])?;
                    ensure!(
                        word(&f, 0xcc + 0x10) == self.base.unwrap() as u32,
                        "side constant depth differs: original={} Rust={}",
                        word(&f, 0xdc),
                        self.base.unwrap()
                    );
                }
            }
            Stage::Blit => {
                let rbp = e["side_frame"]["bp"]
                    .as_u64()
                    .context("side blitter BP absent")? as u32;
                let r = self.rect.unwrap();
                let expected = [
                    0x4c8afa,
                    r[0] as u32,
                    r[1] as u32,
                    (r[2] - r[0]) as u32,
                    (r[3] - r[1]) as u32,
                    rbp - 0xa8,
                    word(&s.raw, 8) + 0x420,
                    rbp - 0x90,
                    s.targets.color
                        + ((r[1] as u32) * (s.targets.color_pitch / 2) + r[0] as u32) * 2,
                    s.targets.color_pitch / 2,
                    s.targets.depth
                        + ((r[1] as u32) * (s.targets.depth_pitch / 2) + r[0] as u32) * 2,
                    s.targets.depth_pitch / 2,
                    rbp - 0xb8,
                    self.base.unwrap() as u32,
                    q["context"]["pointer"].as_u64().unwrap() as u32,
                ];
                ensure!(
                    expected
                        .iter()
                        .enumerate()
                        .all(|(i, v)| word(&stack, i * 4) == *v),
                    "computed side blitter arguments differ"
                );
            }
            Stage::Returned => ensure!(
                std::array::from_fn::<_, 4, _>(|i| word(
                    &frame,
                    if self.direct.is_some() {0xa0-0x78+i*4} else {0x178
                        - (if self.params.alternate == 0 {
                            0x160
                        } else {
                            0x140
                        })
                        + i * 4}
                ) as i32)
                    == self.rect.unwrap(),
                "side returned rectangle differs"
            ),
            _ => (),
        }
        if matches!(
            self.stage,
            Stage::SetupReturned
                | Stage::Clip
                | Stage::Clipped
                | Stage::RectReturned
                | Stage::PointDepth
                | Stage::PointDepthReturned
                | Stage::ColorEnabled
                | Stage::DepthEnabled
                | Stage::Blit
                | Stage::BlitReturned
        ) {
            let f = decode(&e["side_frame"]["raw"])?;
            let r = self.rect.unwrap();
            let packed = [r[1] as i16, r[0] as i16, r[3] as i16, r[2] as i16]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<_>>();
            ensure!(
                word(&f, 0x54) == frame_pointer + 10
                    && word(&f, 0x58) == frame_pointer + 10
                    && f[0x5c..0x64] == packed
                    && word(&f, 0x80) == word(&s.raw, 8)
                    && word(&f, 0x84) == word(&s.raw, 8) + 0x420,
                "side initialized constructor fields differ"
            );
        }
        s.compare(q)
    }
}
