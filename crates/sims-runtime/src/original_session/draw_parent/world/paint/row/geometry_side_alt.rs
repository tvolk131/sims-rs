//!4c8b1f/49ecbc: alternate side, two indexed streams and eight depth groups.
use super::*;
use sims_sim::lot_presenter::{dual_sprite as raster, sprite_full, terrain_raster::Data};
const INPUT: &str = "bae42ba9a81c7d8e6508ffd7de0e30b625e660cfe982ffaf79bbd47dad420d89";
const COLOR: &str = "67e5b0c70054ec04e9e23d2e4f02e19b0bff2939bddd63e14ce214ddca819f9c";
const MASK: &str = "cb74200ef284ceef9e47e46af325f02dd717f5ca71ab2921f074f9c86f9ee8a3";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Sprite {
    image: u32,
    resource: u32,
    raw: Vec<u8>,
    frame: u32,
    encoded: Vec<u8>,
}
impl Sprite {
    fn import(raw: &[u8], image: u32) -> Result<(u32, Self)> {
        let v: serde_json::Value = serde_json::from_slice(raw)?;
        let m = Memory::new(&v)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "alternate sprite epoch differs"
        );
        let r = &v["resources"][0];
        ensure!(
            r["image"] == image && r["resident"] == true && r["frame_index"] == 9,
            "alternate frame was not resident in B"
        );
        let cache = r["palette"].as_u64().context("alternate cache absent")? as u32;
        let resource = r["resource"]
            .as_u64()
            .context("alternate resource absent")? as u32;
        let header = m.read(resource, 44)?;
        let c = m.read(cache, 128)?;
        let (a, z) = (word(&c, 8), word(&c, 12));
        ensure!(a < z && (z - a) % 4 == 0, "alternate cache extent differs");
        let mut p = word(&m.read(a + image % ((z - a) / 4) * 4, 4)?, 0);
        let mut found = 0;
        let mut seen = std::collections::BTreeSet::new();
        while p != 0 {
            ensure!(seen.insert(p) && seen.len() < 2048, "alternate cache cycle");
            let n = m.read(p, 12)?;
            if word(&n, 4) == image {
                found = word(&n, 8);
                break;
            }
            p = word(&n, 0);
        }
        ensure!(
            found == resource && word(&header, 4) > 9,
            "alternate cache/resource ownership differs"
        );
        let frame = word(&m.read(word(&header, 12) + 36, 4)?, 0);
        ensure!(r["frame"] == frame, "alternate frame alias differs");
        let bytes = m.read(frame, 8192)?;
        let used = raster::frame_bytes(&bytes).map_err(anyhow::Error::msg)?;
        Ok((
            cache,
            Self {
                image,
                resource,
                raw: header,
                frame,
                encoded: bytes[..used].to_vec(),
            },
        ))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    source: String,
    cache: u32,
    mask: Sprite,
    color: Sprite,
    palette: Vec<u8>,
    palette_header: Vec<u8>,
    depth: Vec<u8>,
    curve: Vec<i16>,
    columns: Vec<u16>,
    zero_columns: Vec<u16>,
    map: Vec<u8>,
    mask_scratch: Vec<u8>,
    index_scratch: Vec<u8>,
    targets: sims_vm::scheduler::clock::draw::targets::Buffers,
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("alternate bytes absent")?,
    )
}
fn short(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
impl Source {
    pub fn import(
        input: &[u8],
        color: &[u8],
        mask: &[u8],
        targets: sims_vm::scheduler::clock::draw::targets::Buffers,
    ) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == INPUT
                && sims_fixtures::digest(color) == COLOR
                && sims_fixtures::digest(mask) == MASK,
            "alternate B input differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "alternate B epoch differs"
        );
        let m = Memory::new(&v)?;
        let (cache, color) = Sprite::import(color, 5120)?;
        let (other, mask) = Sprite::import(mask, 6144)?;
        ensure!(
            cache == other && m.read(color.resource, 44)? == color.raw,
            "alternate cache/physical aliases differ"
        );
        let owner = word(&color.raw, 8);
        let depth = m.read(0x9e88960 + 0xb8, 0xb0)?;
        let zoom = word(&m.read(0x6f00d0, 16)?, 8);
        ensure!(zoom == 1, "alternate other zoom is pending");
        let s = Self {
            source: INPUT.into(),
            cache,
            mask,
            color,
            palette: m.read(owner + 0x420, 512)?,
            palette_header: m.read(owner, 32)?,
            curve: m
                .read(word(&depth, 4 + (3 - zoom) as usize * 4) - 256, 2048)?
                .chunks_exact(2)
                .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
                .collect(),
            columns: m
                .read(word(&depth, (14 - zoom) as usize * 4) - 256, 1024)?
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .collect(),
            depth,
            zero_columns: m
                .read(0x668540, 64)?
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .collect(),
            map: m.read(0x668440, 256)?,
            mask_scratch: m.read(0x6c6674, 32)?,
            index_scratch: m.read(0x6edec0, 32)?,
            targets,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == INPUT
                && self.mask.encoded.len() >= 10
                && self.color.encoded.len() >= 10
                && self.mask.image == 6144
                && self.color.image == 5120
                && self.mask.encoded[..8] == self.color.encoded[..8]
                && self.palette.len() == 512
                && self.palette_header.len() == 32
                && self.depth.len() == 0xb0
                && self.curve.len() == 1024
                && self.columns.len() == 512
                && self.zero_columns.len() == 32
                && self.map.len() == 256
                && self.mask_scratch.len() == 32
                && self.index_scratch.len() == 32,
            "alternate retained source differs"
        );
        Ok(())
    }
    pub fn compare(&self, q: &serde_json::Value) -> Result<()> {
        let e = &q["visible_geometry"];
        if e.get("side_alt_frame").is_some() {
            if let Some(s) = e.get("scratch") {
                ensure!(
                    decode(&s["mask"])? == self.mask_scratch,
                    "alternate coverage scratch differs"
                );
                ensure!(
                    decode(&s["indices"])? == self.index_scratch,
                    "alternate color-index scratch differs"
                );
            }
        }
        Ok(())
    }
    fn plan(&self, j: &Job, c: &Frame, d: &Data) -> Result<raster::ConstantPlan> {
        let r = j.rect.context("alternate rectangle absent")?;
        let w = c.state().words();
        let z = w[8];
        ensure!(
            z == 1 && w[17] == 0x9e88960 + 0xb8,
            "alternate current depth/zoom differs"
        );
        let width = usize::try_from(r[2] - r[0])?;
        let height = usize::try_from(r[3] - r[1])?;
        let rows = (0..height)
            .map(|y| {
                let at = (w[9] as i32)
                    .wrapping_add(8i32.wrapping_shl(z))
                    .wrapping_sub(r[1] + y as i32)
                    .wrapping_add(128);
                Ok(i32::from(
                    *self
                        .curve
                        .get(usize::try_from(at)?)
                        .context("alternate curve outside B span")?,
                )
                .wrapping_add(w[10] as i32))
            })
            .collect::<Result<Vec<_>>>()?;
        let base = 128 + (16usize << z);
        let half = 8usize << z;
        let a = self
            .columns
            .get(base - half..base - half + width)
            .context("alternate first columns absent")?;
        let b = self
            .columns
            .get(base - half - 1..base - half - 1 + width)
            .context("alternate second columns absent")?;
        let same = self
            .columns
            .get(base..base + width)
            .context("alternate columns absent")?;
        let zero = self
            .zero_columns
            .get(..width)
            .context("alternate constant columns absent")?;
        let full = word(&self.depth, (18 - z) as usize * 4) as i32;
        let half = full / 2;
        raster::plan_mapped(raster::MappedInput {
            mask: &self.mask.encoded,
            color: &self.color.encoded,
            palette: &self.palette,
            map: &self.map,
            columns: [a, b, same, same, zero, zero, zero, zero],
            offsets: [full, full, 0, 0, half, full, half, full],
            rows: &rows,
            screen: [r[0], r[1]],
            margins: j.margins,
            color_pitch: (self.targets.color_pitch / 2) as i32,
            depth_pitch: (self.targets.depth_pitch / 2) as i32,
            mask_scratch: &self.mask_scratch,
            index_scratch: &self.index_scratch,
            depth: &d.depth,
        })
        .map_err(anyhow::Error::msg)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub cache: u32,
    pub color_image: u32,
    pub coverage_image: u32,
    pub index: u32,
    pub screen: [i32; 2],
    pub clip: [i32; 4],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    ResourceOne,
    ResourceOneReturned,
    ResourceTwo,
    ResourceTwoReturned,
    FrameOne,
    FrameOneReturned,
    FrameTwo,
    FrameTwoReturned,
    Setup,
    Palette,
    PaletteReturned,
    SetupReturned,
    Clip,
    Clipped,
    Zoom,
    ZoomReturned,
    Rows,
    RowsReturned,
    Rect,
    RectReturned,
    Color,
    ColorReturned,
    Depth,
    DepthReturned,
    ZoomAgain,
    ZoomAgainReturned,
    DepthOwner,
    DepthOwnerReturned,
    Blit,
    Drawing,
    BlitReturned,
    Returned,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    params: Params,
    stage: Stage,
    rect: Option<[i32; 4]>,
    margins: [i32; 4],
    clipped: bool,
    plan: Option<raster::ConstantPlan>,
    next: usize,
}
impl Job {
    pub fn new(params: Params) -> Result<Self> {
        ensure!(
            params.color_image == 5120 && params.coverage_image == 6144 && params.index == 9,
            "alternate sprite/frame branch is pending"
        );
        Ok(Self {
            params,
            stage: Stage::Entry,
            rect: None,
            margins: [0; 4],
            clipped: false,
            plan: None,
            next: 0,
        })
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    pub fn rectangle(&self) -> Result<[i32; 4]> {
        self.rect.context("alternate returned rectangle absent")
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.params.index == 9 && self.next <= self.plan.as_ref().map_or(0, |p| p.writes.len()),
            "alternate saved continuation differs"
        );
        Ok(())
    }
    pub fn at_cut(&self, pc: u32) -> bool {
        matches!(
            (pc, self.stage),
            (0x4c8b1f, Stage::Entry)
                | (0x4c8e0d, Stage::ResourceOne)
                | (0x4c8b4e, Stage::ResourceOneReturned)
                | (0x4c8e0d, Stage::ResourceTwo)
                | (0x4c8b5a, Stage::ResourceTwoReturned)
                | (0x4c19b1, Stage::FrameOne)
                | (0x4c8b78, Stage::FrameOneReturned)
                | (0x4c19b1, Stage::FrameTwo)
                | (0x4c8b84, Stage::FrameTwoReturned)
                | (0x4c8bab, Stage::Setup)
                | (0x4c7fdb, Stage::Setup)
                | (0x4c76d1, Stage::Setup)
                | (0x4b954a, Stage::Palette)
                | (0x4c77a1, Stage::PaletteReturned)
                | (0x4c8bb0, Stage::SetupReturned)
                | (0x4c8be7, Stage::Clip)
                | (0x4c77bc, Stage::Clip)
                | (0x4c8bec, Stage::Clipped)
                | (0x4c8c5d, Stage::Zoom)
                | (0x492b67, Stage::Zoom)
                | (0x4c8c62, Stage::ZoomReturned)
                | (0x4c8c69, Stage::Rows)
                | (0x4c8c6e, Stage::RowsReturned)
                | (0x4c8cc5, Stage::Rect)
                | (0x4c8ccb, Stage::RectReturned)
                | (0x4b444f, Stage::Color)
                | (0x4c8cd2, Stage::ColorReturned)
                | (0x4b4456, Stage::Depth)
                | (0x4c8cdd, Stage::DepthReturned)
                | (0x492b67, Stage::ZoomAgain)
                | (0x4c8ce8, Stage::ZoomAgainReturned)
                | (0x4b4513, Stage::DepthOwner)
                | (0x4c8cf2, Stage::DepthOwnerReturned)
                | (0x4c8d32, Stage::Blit)
                | (0x49ecbc, Stage::Blit)
                | (0x4c8d37, Stage::BlitReturned)
                | (0x53e3b9, Stage::Returned)
        )
    }
    pub(super) fn advance(&mut self, s: &mut Source, d: &mut Data, c: &Frame) -> Result<bool> {
        self.validate()?;
        s.validate()?;
        ensure!(
            self.params.cache == s.cache
                && d.color_pitch == s.targets.color_pitch as i32
                && d.depth_pitch == s.targets.depth_pitch as i32,
            "alternate shared cache/target identity differs"
        );
        self.stage = match self.stage {
            Stage::Entry => Stage::ResourceOne,
            Stage::ResourceOne => Stage::ResourceOneReturned,
            Stage::ResourceOneReturned => Stage::ResourceTwo,
            Stage::ResourceTwo => Stage::ResourceTwoReturned,
            Stage::ResourceTwoReturned => Stage::FrameOne,
            Stage::FrameOne => Stage::FrameOneReturned,
            Stage::FrameOneReturned => Stage::FrameTwo,
            Stage::FrameTwo => Stage::FrameTwoReturned,
            Stage::FrameTwoReturned => Stage::Setup,
            Stage::Setup => {
                let packed = c
                    .state()
                    .words()
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>();
                ensure!(
                    packed[0x35..0x39] == [0; 4],
                    "alternate modified palette controls are pending"
                );
                let [x, y] = self.params.screen;
                let b = &s.mask.encoded;
                self.rect = Some([
                    i32::from(short(b, 2).wrapping_add(x as i16)),
                    i32::from(short(b, 0).wrapping_add(y as i16)),
                    i32::from(short(b, 6).wrapping_add(x as i16)),
                    i32::from(short(b, 4).wrapping_add(y as i16)),
                ]);
                Stage::Palette
            }
            Stage::Palette => {
                ensure!(
                    c.state().words()[18] == 1f32.to_bits(),
                    "alternate palette modification is pending"
                );
                Stage::PaletteReturned
            }
            Stage::PaletteReturned => Stage::SetupReturned,
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
                    Stage::Zoom
                }
            }
            Stage::Zoom => Stage::ZoomReturned,
            Stage::ZoomReturned => Stage::Rows,
            Stage::Rows => Stage::RowsReturned,
            Stage::RowsReturned => Stage::Rect,
            Stage::Rect => Stage::RectReturned,
            Stage::RectReturned => Stage::Color,
            Stage::Color => {
                ensure!(
                    c.state().flags(0).map_err(anyhow::Error::msg)? & 1 != 0,
                    "alternate disabled color is pending"
                );
                Stage::ColorReturned
            }
            Stage::ColorReturned => Stage::Depth,
            Stage::Depth => {
                ensure!(
                    c.state().flags(0).map_err(anyhow::Error::msg)? & 2 != 0,
                    "alternate disabled depth is pending"
                );
                Stage::DepthReturned
            }
            Stage::DepthReturned => Stage::ZoomAgain,
            Stage::ZoomAgain => Stage::ZoomAgainReturned,
            Stage::ZoomAgainReturned => Stage::DepthOwner,
            Stage::DepthOwner => Stage::DepthOwnerReturned,
            Stage::DepthOwnerReturned => Stage::Blit,
            Stage::Blit => {
                self.plan = Some(s.plan(self, c, d)?);
                Stage::Drawing
            }
            Stage::Drawing => {
                let p = self.plan.as_ref().unwrap();
                if let Some(w) = p.writes.get(self.next) {
                    sprite_full::apply(w, &mut d.color, &mut d.depth)
                        .map_err(anyhow::Error::msg)?;
                    self.next += 1;
                    Stage::Drawing
                } else {
                    s.mask_scratch.clone_from(&p.mask_scratch);
                    s.index_scratch.clone_from(&p.index_scratch);
                    Stage::BlitReturned
                }
            }
            Stage::BlitReturned => Stage::Returned,
            Stage::Returned => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
    pub(super) fn compare(&self, q: &serde_json::Value, s: &Source, c: &Frame) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("alternate PC absent")? as u32;
        ensure!(self.at_cut(pc), "alternate renderer stage differs");
        let e = &q["visible_geometry"];
        let stack = decode(&e["stack"]["raw"])?;
        let bp = e["side_alt_frame"]["bp"].as_u64().unwrap_or(0) as u32;
        let ctx = q["context"]["pointer"]
            .as_u64()
            .context("alternate context absent")? as u32;
        match self.stage {
            Stage::Entry => {
                let p = &self.params;
                let expected = [
                    p.color_image,
                    p.coverage_image,
                    p.index,
                    p.screen[0] as u32,
                    p.screen[1] as u32,
                    0,
                    ctx,
                    0,
                ];
                ensure!(
                    q["registers"]["ecx"] == p.cache
                        && expected
                            .iter()
                            .enumerate()
                            .all(|(i, v)| word(&stack, 8 + i * 4) == *v),
                    "alternate entry arguments differ"
                );
            }
            Stage::ResourceOne | Stage::ResourceTwo => {
                let r = if self.stage == Stage::ResourceOne {
                    &s.mask
                } else {
                    &s.color
                };
                ensure!(
                    q["registers"]["ecx"] == s.cache && word(&stack, 4) == r.image,
                    "alternate resource query differs"
                );
            }
            Stage::ResourceOneReturned | Stage::ResourceTwoReturned => {
                let r = if self.stage == Stage::ResourceOneReturned {
                    &s.mask
                } else {
                    &s.color
                };
                ensure!(
                    q["registers"]["eax"] == r.resource,
                    "alternate resource result differs"
                );
            }
            Stage::FrameOne | Stage::FrameTwo => {
                let r = if self.stage == Stage::FrameOne {
                    &s.mask
                } else {
                    &s.color
                };
                ensure!(
                    q["registers"]["ecx"] == r.resource && word(&stack, 4) == 9,
                    "alternate frame query differs"
                );
            }
            Stage::FrameOneReturned | Stage::FrameTwoReturned => {
                let r = if self.stage == Stage::FrameOneReturned {
                    &s.mask
                } else {
                    &s.color
                };
                ensure!(
                    q["registers"]["eax"] == r.frame,
                    "alternate frame result differs"
                );
            }
            Stage::Setup => {
                ensure!(
                    q["registers"]["ecx"] == bp - 0x68,
                    "alternate constructor receiver differs"
                );
                let p = &self.params;
                let expected = if pc == 0x4c76d1 {
                    vec![
                        s.color.frame + 10,
                        s.mask.frame + 10,
                        word(&s.color.raw, 8),
                        word(&s.color.raw, 32),
                        s.mask.frame,
                        p.screen[0] as u32,
                        p.screen[1] as u32,
                        ctx,
                        0,
                        1,
                    ]
                } else {
                    vec![
                        s.mask.frame + 10,
                        s.color.frame + 10,
                        word(&s.color.raw, 8),
                        word(&s.color.raw, 32),
                        s.mask.frame,
                        s.color.frame,
                        p.screen[0] as u32,
                        p.screen[1] as u32,
                        ctx,
                        0,
                        0,
                    ]
                };
                let offset = usize::from(pc != 0x4c8bab) * 4;
                ensure!(
                    expected
                        .iter()
                        .enumerate()
                        .all(|(i, v)| if pc == 0x4c76d1 && i == 8 {
                            word(&stack, offset + 4 * i) & 65535 == *v
                        } else {
                            word(&stack, offset + 4 * i) == *v
                        }),
                    "alternate constructor ABI differs"
                );
            }
            Stage::Palette => {
                ensure!(
                    q["registers"]["ecx"] == word(&s.color.raw, 8)
                        && word(&stack, 4) == 1f32.to_bits(),
                    "alternate palette query differs"
                );
                let p = &e["palette_selection"];
                ensure!(
                    decode(&p["raw"])? == s.palette_header
                        && decode(&p["unit_palette"])? == s.palette,
                    "alternate B palette differs"
                );
            }
            Stage::PaletteReturned => ensure!(
                q["registers"]["eax"] == word(&s.color.raw, 8) + 0x420,
                "alternate palette result differs"
            ),
            Stage::SetupReturned => {
                let f = decode(&e["side_alt_frame"]["raw"])?;
                ensure!(
                    f.len() == 0xec,
                    "alternate initialized frame extent differs"
                );
                ensure!(
                    word(&f, 0x50) == s.color.frame + 10
                        && word(&f, 0x54) == s.mask.frame + 10
                        && word(&f, 0x7c) == word(&s.color.raw, 8)
                        && word(&f, 0x80) == word(&s.color.raw, 8) + 0x420,
                    "alternate initialized stream/palette pointers differ"
                );
                ensure!(
                    self.rect
                        == Some([
                            i32::from(short(&f, 0x5a)),
                            i32::from(short(&f, 0x58)),
                            i32::from(short(&f, 0x5e)),
                            i32::from(short(&f, 0x5c))
                        ]),
                    "alternate initialized rectangle differs"
                );
            }
            Stage::Clipped => ensure!(
                q["registers"]["eax"]
                    .as_u64()
                    .context("alternate clip AL absent")?
                    & 255
                    == u64::from(self.clipped),
                "alternate clip result differs"
            ),
            Stage::ZoomReturned | Stage::ZoomAgainReturned => ensure!(
                q["registers"]["eax"] == c.state().words()[8],
                "alternate zoom result differs"
            ),
            Stage::DepthOwnerReturned => ensure!(
                q["registers"]["eax"] == c.state().words()[17],
                "alternate depth owner result differs"
            ),
            Stage::Blit => {
                let r = self.rect.unwrap();
                let expected = [
                    r[0] as u32,
                    r[1] as u32,
                    (r[2] - r[0]) as u32,
                    (r[3] - r[1]) as u32,
                    bp - 0x80,
                    word(&s.color.raw, 8) + 0x420,
                    bp - 0x98,
                    s.targets
                        .color
                        .wrapping_add((r[1] as u32).wrapping_mul(s.targets.color_pitch))
                        .wrapping_add((r[0] as u32).wrapping_mul(2)),
                    s.targets.color_pitch / 2,
                    s.targets
                        .depth
                        .wrapping_add((r[1] as u32).wrapping_mul(s.targets.depth_pitch))
                        .wrapping_add((r[0] as u32).wrapping_mul(2)),
                    s.targets.depth_pitch / 2,
                    bp - 0xa8,
                    word(&s.depth, (14 - c.state().words()[8]) as usize * 4)
                        + (16 << c.state().words()[8]) * 2,
                    ctx,
                ];
                let offset = usize::from(pc == 0x49ecbc) * 4;
                ensure!(
                    expected
                        .iter()
                        .enumerate()
                        .all(|(i, v)| word(&stack, offset + 4 * i) == *v),
                    "alternate blitter ABI differs"
                );
            }
            _ => (),
        }
        s.compare(q)
    }
}
