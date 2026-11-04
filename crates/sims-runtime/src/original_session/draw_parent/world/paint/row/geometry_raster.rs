//! Retained B resource/scratch owners and the ordinary 4c79ca renderer child.
//! Call arguments come from the Rust visible continuation, never a C8 reply.
use super::*;
use sims_sim::lot_presenter::{dual_sprite, sprite_full, terrain_raster::Data};
use sims_vm::scheduler::clock::draw::targets::Buffers;
use std::collections::BTreeMap;
const INPUT: &str = "d777c108ff6b83f26b80a60607b80922b93dd1741b2892d70a96618e57d36d9f";
const MATERIAL: &str = "c7d259decfb58e83a2c71f3851c4abfe52203f7518ec582a6c91d102b9cf94e8";
const PLAIN_INPUT: &str = "817c64767b69990ee58d0897453ce2a6ae2696c61151a56d37edebae8f1847fa";
const PLAIN_MATERIAL: &str = "cc11ed5c053784f0c6c2c61b9496652a19ff10ffaa7654da530405dfa057bd5e";
const EDGE_INPUT: &str = "9e486037e7ccf9d110d44a62406f28c17fd8efe7a1e4b4c95c20d802b4e0e1f8";
const EDGE_MATERIAL: &str = "a72aafcbbf612f0823a60b5ed45842cd91f2d780b62c0534c0bfa33d3bac3e4d";
const COVER_INPUT: &str = "82863b48aade7d95a02bbd6877cfa472881c4c0f9648921a91568e8b2a1c2c81";
const COVER_MATERIAL: &str = "a90bfea7761bc33645e87d380dbc665738a6b5124e94d4ac38337d713199e880";
const OVERLAY_INPUT: &str = "ff0b6e5fe672f46281ce3f0b1fc6c5e6ba39aac38335d2d7be220689c9435ef6";
const OVERLAY_MATERIAL: &str = "41473a199507705b07746c88b76c8171a69461604bf2f1460bdee9df89c23dfb";
const NEXT_RASTERS: &[(&str, &str)] = &[
    (
        "c8e7e5e7725d85ac5860273ce367bd413ea3f2e88c3416ce7b2d995c046a14d5",
        "35ea8004feae810b260e1bfb5014f9361af20ed2a7064b5463e8ff3d5adab978",
    ),
    (
        "6d6278df58952331a8e564e4babec8a3733c4c7931a186b62024c7db1731303b",
        "45ad496e7f14df51785805ef26d1fba35f28f16abcd95a90632faa5dc0e3b25e",
    ),
    (
        "a9532d17418a7e115712567767e67720d4ffae1a513ba2f0792c486d7398614d",
        "71ce04947cd3572c52e8beb89a0dac155373bb59d2eb37c41f4827e35b4e18da",
    ),
    (
        "a698a3216211787af95e83f554042947f666730b5bd74cea29d30eeff5115806",
        "a260ff2716ac68bd27820473abfd941f4582fa727c75dd716d49cdbf59224be2",
    ),
];
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Palette {
    owner: u32,
    header: Vec<u8>,
    adjusted: Vec<u8>,
    generation: u32,
    unit_light: u32,
    epsilon: u32,
}
impl Palette {
    ///4b954a tests unit light first, then generation, then cached brightness.
    ///The existing half-brightness table is B state, not a renderer result.
    fn select(&self, light: u32) -> Result<u32> {
        let light = f64::from(f32::from_bits(light));
        let unit = f64::from(f32::from_bits(self.unit_light));
        let epsilon = f64::from(f32::from_bits(self.epsilon));
        ensure!(
            light.is_finite(),
            "geometry palette nonfinite light is pending"
        );
        if (light - unit).abs() <= epsilon {
            return Ok(self.owner + 0x420);
        }
        ensure!(
            word(&self.header, 0x1c) >= self.generation,
            "geometry palette generation refresh is pending"
        );
        let cached = f64::from(f32::from_bits(word(&self.header, 0x18)));
        ensure!(
            cached.is_finite(),
            "geometry cached palette nonfinite light is pending"
        );
        ensure!(
            (light - cached).abs() <= epsilon,
            "geometry palette brightness update is pending"
        );
        Ok(self.owner + 0xc20)
    }
}
fn default_frame_index() -> u32 {
    1
}
fn is_default_frame_index(i: &u32) -> bool {
    *i == 1
}
const DIAGONAL_INPUT: &str = "6b151a5c63ed122ff0cc60c1d938a3c66c9a27173cf7bfeaecdbbef6ec95091b";
const DIAGONAL_WIDE_INPUT: &str = "592b8e84fd905167a7ee7d497b2e977808f97e2be583756e14f92f8a60337d0b";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DiagonalColumns {
    source: String,
    pointer: u32,
    values: Vec<u16>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Sprite {
    #[serde(
        default = "default_frame_index",
        skip_serializing_if = "is_default_frame_index"
    )]
    index: u32,
    cache: u32,
    image: u32,
    resource: u32,
    directory: u32,
    frame: u32,
    raw: Vec<u8>,
    encoded: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    source: String,
    sprites: Vec<Sprite>,
    palette: Vec<u8>,
    palette_address: u32,
    depth_owner: u32,
    depth: Vec<u8>,
    curve: Vec<i16>,
    columns: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagonal_columns: Option<DiagonalColumns>,
    zoom: u32,
    pub targets: Buffers,
    mask_scratch: Vec<u8>,
    color_scratch: Vec<u8>,
    shading: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    palette_selection: Option<Palette>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra: Option<Box<Source>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub coverage_cache: u32,
    #[serde(default, skip_serializing_if = "zero_override")]
    pub coverage_override: u32,
    pub coverage_image: u32,
    pub wallpaper_cache: u32,
    pub wallpaper_image: u32,
    pub index: u32,
    pub point: [i32; 2],
    pub offset: i32,
    pub mask: u32,
    pub clip: [i32; 4],
    pub descriptor: [u8; 4],
}
fn zero_override(v: &u32) -> bool {
    *v == 0
}
impl Params {
    fn mask_cache(&self) -> u32 {
        if self.coverage_override == 0 {
            self.coverage_cache
        } else {
            self.coverage_override
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    ResourceFirst,
    FirstReturned,
    ResourceSecond,
    SecondReturned,
    FrameFirst,
    FrameFirstReturned,
    FrameSecond,
    FrameSecondReturned,
    Setup,
    SetupReturned,
    Clip,
    ClipReturned,
    Rect,
    RectReturned,
    BlitCall,
    Blit,
    Drawing,
    BlitReturned,
    Complete,
    Shade,
    Palette,
    PaletteReturned,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    params: Params,
    stage: Stage,
    rectangle: Option<[i32; 4]>,
    margins: [i32; 4],
    clipped: Option<bool>,
    plan: Option<dual_sprite::Plan>,
    next: usize,
}
fn short(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
impl Source {
    pub fn import(encoded: &[u8], material: &[u8], targets: Buffers) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        let material_source = sims_fixtures::digest(material);
        ensure!(
            (source == INPUT && material_source == MATERIAL)
                || (source == PLAIN_INPUT && material_source == PLAIN_MATERIAL)
                || (source == EDGE_INPUT && material_source == EDGE_MATERIAL)
                || (source == COVER_INPUT && material_source == COVER_MATERIAL)
                || (source == OVERLAY_INPUT && material_source == OVERLAY_MATERIAL)
                || NEXT_RASTERS
                    .iter()
                    .any(|(a, b)| source == *a && material_source == *b),
            "geometry raster source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        let t: serde_json::Value = serde_json::from_slice(material)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && t["tick"] == 89073
                && t["executes_target_code"] == false,
            "geometry raster epoch differs"
        );
        let m = Memory::new(&v)?;
        let tm = Memory::new(&t)?;
        let shading = m.read(0x6f018a, 1)?[0] != 0;
        ensure!(
            sims_fixtures::digest(&m.read(0x4aec97, 18)?)
                == "0dd34da61a1b2e4c1b197f3915097739b237fd7dbfb27f665d2460fc39543247",
            "geometry descriptor count consumer differs"
        );
        let mut sprites = vec![];
        for entry in t["resources"]
            .as_array()
            .context("geometry resources absent")?
        {
            let resource = entry["resource"].as_u64().context("resource absent")? as u32;
            let raw = tm.read(resource, 0x2c)?;
            ensure!(
                m.read(resource, 0x2c)? == raw,
                "geometry resource B aliases differ"
            );
            let cache = entry["palette"].as_u64().context("sprite cache absent")? as u32;
            let image = entry["image"].as_u64().context("sprite key absent")? as u32;
            let c = tm.read(cache, 0x80)?;
            let (a, z) = (word(&c, 8), word(&c, 12));
            ensure!(a < z && (z - a) % 4 == 0, "geometry cache buckets differ");
            let mut p = word(&tm.read(a + (image % ((z - a) / 4)) * 4, 4)?, 0);
            let mut found = None;
            let mut seen = std::collections::BTreeSet::new();
            while p != 0 {
                ensure!(
                    seen.insert(p) && seen.len() < 2048,
                    "geometry sprite cache cycle"
                );
                let n = tm.read(p, 12)?;
                if word(&n, 4) == image {
                    found = Some(word(&n, 8));
                    break;
                }
                p = word(&n, 0);
            }
            ensure!(
                found == Some(resource),
                "geometry sprite was not resident in B"
            );
            let directory = word(&raw, 12);
            let frame = word(&tm.read(directory, 8)?, 4);
            ensure!(
                entry["frame"] == frame && word(&raw, 4) > 1,
                "geometry frame ownership differs"
            );
            let prefix = crate::original_session::completed_animations::bytes(
                entry["frame_prefix"]
                    .as_str()
                    .context("frame prefix absent")?,
            )?;
            let size = dual_sprite::frame_bytes(&prefix).map_err(anyhow::Error::msg)?;
            let body = tm.read(frame, size)?;
            ensure!(prefix[..size] == body, "geometry frame bytes differ");
            sprites.push(Sprite {
                index: 1,
                cache,
                image,
                resource,
                directory,
                frame,
                raw,
                encoded: body,
            });
        }
        let wallpaper = if source == OVERLAY_INPUT || source == NEXT_RASTERS[3].0 {
            &t["resources"][0]["image"]
        } else {
            &t["material"]["images"][0]
        }
        .as_u64()
        .context("wallpaper key absent")? as u32;
        let color = sprites
            .iter()
            .find(|s| s.image == wallpaper)
            .context("wallpaper absent")?;
        let palette_address = word(&color.raw, 8) + 0x420;
        let palette = m.read(palette_address, 512)?;
        let zoom = word(&m.read(0x6f00d0, 16)?, 8);
        ensure!((1..=3).contains(&zoom), "geometry zoom invalid");
        let depth_owner = v["depth_owner"].as_u64().context("depth owner absent")? as u32;
        let depth = m.read(depth_owner, 0xb0)?;
        let curve = m
            .read(word(&depth, 4 + (3 - zoom) as usize * 4) - 256, 2048)?
            .chunks_exact(2)
            .map(|r| i16::from_le_bytes(r.try_into().unwrap()))
            .collect();
        let columns = m
            .read(word(&depth, (14 - zoom) as usize * 4) - 256, 1024)?
            .chunks_exact(2)
            .map(|r| u16::from_le_bytes(r.try_into().unwrap()))
            .collect();
        let palette_selection = if source != INPUT {
            let owner = word(&color.raw, 8);
            let raw = m.read(owner, 0x1000)?;
            Some(Palette {
                owner,
                header: raw[..0x20].to_vec(),
                adjusted: raw[0xc20..0xe20].to_vec(),
                generation: word(&m.read(0x6f0184, 4)?, 0),
                unit_light: word(&m.read(0x666748, 4)?, 0),
                epsilon: word(&m.read(0x67343c, 4)?, 0),
            })
        } else {
            None
        };
        let s = Self {
            source,
            sprites,
            palette,
            palette_address,
            depth_owner,
            depth,
            curve,
            columns,
            diagonal_columns: None,
            zoom,
            targets,
            mask_scratch: m.read(0x6c76b8, 16)?,
            color_scratch: m.read(0x6f0ae8, 32)?,
            shading,
            palette_selection,
            extra: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(d) = &self.diagonal_columns {
            ensure!(self.depth.len() == 0xb0 && matches!(d.source.as_str(), DIAGONAL_INPUT | DIAGONAL_WIDE_INPUT) && d.pointer == word(&self.depth, 0x38)
                && d.values.len() == 64, "geometry diagonal depth owner differs");
        }
        ensure!(
            (matches!(
                self.source.as_str(),
                INPUT | PLAIN_INPUT | EDGE_INPUT | COVER_INPUT | OVERLAY_INPUT
            ) || NEXT_RASTERS.iter().any(|(a, _)| self.source == *a)
                || super::geometry_catalog::is_source(&self.source))
                && (self.sprites.len() == 2
                    || super::geometry_catalog::is_source(&self.source)
                        && !self.sprites.is_empty())
                && self.palette.len() == 512
                && self.curve.len() == 1024
                && self.columns.len() == 512
                && self.depth.len() == 0xb0
                && (self.mask_scratch.len() == 16 && self.color_scratch.len() == 32
                    || self.mask_scratch.len() == 64 && self.color_scratch.len() == 128
                        && self.diagonal_columns.as_ref().is_some_and(|d| d.source == DIAGONAL_WIDE_INPUT)),
            "geometry raster retained owner differs"
        );
        ensure!(
            (self.source != INPUT) == self.palette_selection.is_some(),
            "geometry palette owner profile differs"
        );
        if let Some(p) = &self.palette_selection {
            ensure!(
                p.header.len() == 0x20
                    && p.adjusted.len() == 512
                    && p.owner + 0x420 == self.palette_address
                    && p.unit_light == 1f32.to_bits()
                    && f32::from_bits(p.epsilon).is_finite()
                    && f32::from_bits(p.epsilon) > 0.,
                "geometry palette selector retained state differs"
            );
        }
        if let Some(e) = &self.extra {
            e.validate()?;
            ensure!(
                self.targets == e.targets
                    && self.depth == e.depth
                    && self.curve == e.curve
                    && self.columns == e.columns
                    && self.zoom == e.zoom
                    && self.shading == e.shading,
                "geometry added resource aliases differ"
            );
        }
        ensure!(
            self.targets.color_pitch > 0
                && self.targets.color_pitch % 2 == 0
                && self.targets.depth_pitch > 0
                && self.targets.depth_pitch % 2 == 0,
            "geometry target pitches differ"
        );
        Ok(())
    }
    pub(super) fn bind_diagonal_columns(&mut self, encoded: &[u8]) -> Result<()> {
        let source = sims_fixtures::digest(encoded);
        ensure!(matches!(source.as_str(), DIAGONAL_INPUT | DIAGONAL_WIDE_INPUT),
            "geometry diagonal B source differs");
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(v["tick"] == 89073 && v["executes_target_code"] == false
            && v["depth_owner"] == self.depth_owner, "geometry diagonal B epoch differs");
        let m = Memory::new(&v)?;
        ensure!(m.read(self.depth_owner, 0xb0)? == self.depth
            && word(&m.read(0x6f00d0, 16)?, 8) == self.zoom,
            "geometry diagonal depth/projection aliases differ");
        let pointer = word(&self.depth, 0x38);
        let values = m.read(pointer, 128)?.chunks_exact(2)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap())).collect();
        let d = DiagonalColumns { source, pointer, values };
        ensure!(self.diagonal_columns.as_ref().is_none_or(|old|
            old.pointer == d.pointer && old.values == d.values
                && (old.source == d.source || old.source == DIAGONAL_INPUT && d.source == DIAGONAL_WIDE_INPUT)),
            "geometry diagonal binding would change retained input");
        if d.source == DIAGONAL_WIDE_INPUT {
            let mask = m.read(0x6c76b8, 64)?;
            let color = m.read(0x6f0ae8, 128)?;
            //Only newly owned tail bytes come from B; the hot prefix has
            //already advanced with the enclosing retained renderer.
            self.mask_scratch.extend_from_slice(&mask[self.mask_scratch.len()..]);
            self.color_scratch.extend_from_slice(&color[self.color_scratch.len()..]);
        }
        self.diagonal_columns = Some(d);
        self.validate()
    }
    pub fn extend(&mut self, encoded: &[u8], material: &[u8]) -> Result<()> {
        if let Some(e) = &mut self.extra {
            e.extend(encoded, material)?;
            return self.validate();
        }
        let extra = Self::import(encoded, material, self.targets)?;
        ensure!(
            extra.source != self.source
                && self.depth == extra.depth
                && self.curve == extra.curve
                && self.columns == extra.columns
                && self.zoom == extra.zoom
                && self.shading == extra.shading,
            "geometry added B depth/shading aliases differ"
        );
        //Shared scratch and target planes remain owned by the current caller.
        //The additional B scratch is historical and is never installed.
        self.extra = Some(Box::new(extra));
        self.validate()
    }
    fn find_sprite(&self, cache: u32, image: u32, index: u32) -> Option<&Sprite> {
        self.sprites
            .iter()
            .find(|s| s.cache == cache && s.image == image && s.index == index)
            .or_else(|| {
                self.extra
                    .as_ref()
                    .and_then(|e| e.find_sprite(cache, image, index))
            })
    }
    fn sprite(&self, cache: u32, image: u32, index: u32) -> Result<&Sprite> {
        self.find_sprite(cache, image, index)
            .context("geometry resource loading is pending")
    }
    fn find_palette(&self, address: u32) -> Option<&Self> {
        if self.palette_address == address {
            Some(self)
        } else {
            self.extra.as_deref().and_then(|e| e.find_palette(address))
        }
    }
    fn palette_owner(&self, color: &Sprite) -> Result<&Self> {
        self.find_palette(word(&color.raw, 8) + 0x420)
            .context("geometry color palette owner is pending")
    }
    fn selected_palette(&self, color: &Sprite, context: &Frame) -> Result<(u32, &[u8])> {
        let owner = self.palette_owner(color)?;
        let light = context.state().words()[18];
        if let Some(p) = &owner.palette_selection {
            let address = p.select(light)?;
            return Ok((
                address,
                if address == p.owner + 0x420 {
                    &owner.palette
                } else {
                    &p.adjusted
                },
            ));
        }
        ensure!(
            light == 1f32.to_bits(),
            "geometry original palette brightness update is pending"
        );
        Ok((owner.palette_address, &owner.palette))
    }
    fn compare_palette(&self, q: &serde_json::Value, color: &Sprite) -> Result<()> {
        let source = self.palette_owner(color)?;
        let p = source
            .palette_selection
            .as_ref()
            .context("palette state absent")?;
        let e = &q["visible_geometry"];
        let r = e
            .get("palette_selection")
            .or_else(|| e.get("palette_selection_returned"))
            .context("native palette observation absent")?;
        let decode = |v: &serde_json::Value| {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("palette bytes absent")?,
            )
        };
        ensure!(
            e["palette_generation"] == p.generation
                && r["owner"] == p.owner
                && decode(&r["raw"])? == p.header
                && decode(&r["unit_palette"])? == source.palette
                && decode(&r["adjusted_palette"])? == p.adjusted,
            "retained palette header/table/generation differs"
        );
        Ok(())
    }
    fn pair<'a>(&'a self, p: &Params) -> Result<(&'a Sprite, &'a Sprite)> {
        Ok((
            self.sprite(p.mask_cache(), p.coverage_image, p.index)?,
            self.sprite(
                if p.wallpaper_cache == 0 {
                    p.coverage_cache
                } else {
                    p.wallpaper_cache
                },
                p.wallpaper_image,
                p.index,
            )?,
        ))
    }
    fn plan(&self, j: &Job, c: &Frame, d: &Data) -> Result<dual_sprite::Plan> {
        let (mask, color) = self.pair(&j.params)?;
        let w = c.state().words();
        ensure!(
            w[8] == self.zoom && w[17] == self.depth_owner,
            "geometry current depth receiver differs"
        );
        let rectangle = j.rectangle.context("geometry sprite rectangle absent")?;
        let width = (rectangle[2] - rectangle[0]) as usize;
        let height = (rectangle[3] - rectangle[1]) as usize;
        let rows = (0..height)
            .map(|y| {
                let index = (w[9] as i32)
                    .wrapping_add(8i32.wrapping_shl(self.zoom))
                    .wrapping_sub(rectangle[1].wrapping_add(y as i32));
                let at =
                    usize::try_from(index.checked_add(128).context("geometry curve overflow")?)?;
                Ok(i32::from(
                    *self
                        .curve
                        .get(at)
                        .context("geometry row curve outside retained span")?,
                )
                .wrapping_add(w[10] as i32)
                .wrapping_add(if j.params.mask == 32 { -300 } else { 0 }))
            })
            .collect::<Result<Vec<_>>>()?;
        //4c7bce selects the diagonal table directly; 4c7be2 adds -300 to
        //each row for mask32. Straight walls use the zoom-specific center.
        let diagonal = matches!(j.params.mask, 16 | 32);
        let first = if diagonal { 0 } else {
            128 + (16i32.wrapping_shl(self.zoom)).wrapping_add(j.params.offset)
        };
        let table = if diagonal {
            &self.diagonal_columns.as_ref()
                .context("diagonal geometry B depth columns are unbound")?.values
        } else { &self.columns };
        let columns = (0..width)
            .map(|x| {
                table
                    .get(usize::try_from(
                        first
                            .checked_add(x as i32)
                            .context("geometry column overflow")?,
                    )?)
                    .copied()
                    .context("geometry column outside retained span")
            })
            .collect::<Result<Vec<_>>>()?;
        dual_sprite::plan(dual_sprite::Input {
            mask: &mask.encoded,
            color: &color.encoded,
            palette: self.selected_palette(color, c)?.1,
            screen: [rectangle[0], rectangle[1]],
            margins: j.margins,
            color_pitch: (self.targets.color_pitch / 2) as i32,
            depth_pitch: (self.targets.depth_pitch / 2) as i32,
            rows: &rows,
            columns: &columns,
            mask_scratch: self.mask_scratch.get(..width).context("geometry mask scratch span unbound")?,
            color_scratch: self.color_scratch.get(..width*2).context("geometry color scratch span unbound")?,
            depth: &d.depth,
        })
        .map_err(anyhow::Error::msg)
    }
    pub fn compare(&self, q: &serde_json::Value) -> Result<()> {
        self.validate()?;
        if let Some(flag) = q["visible_geometry"].get("shading_flag") {
            ensure!(
                crate::original_session::completed_animations::bytes(
                    flag.as_str().context("shading flag absent")?
                )? == [u8::from(self.shading)],
                "shared shading control differs"
            );
        }
        if let Some(s) = q["visible_geometry"]
            .get("scratch")
            .filter(|s| s.get("color").is_some())
        {
            let mask = crate::original_session::completed_animations::bytes(
                s["mask"].as_str().context("native mask scratch absent")?)?;
            let color = crate::original_session::completed_animations::bytes(
                s["color"].as_str().context("native color scratch absent")?)?;
            ensure!(color.len() == mask.len()*2
                && self.mask_scratch.get(..mask.len()) == Some(mask.as_slice())
                && self.color_scratch.get(..color.len()) == Some(color.as_slice()),
                "shared geometry scratch rows differ"
            );
        }
        Ok(())
    }
}
impl Job {
    pub fn new(params: Params) -> Result<Self> {
        ensure!(
            matches!(params.mask, 1 | 2) && params.index <= 1
                || matches!((params.mask,params.index),(16,2)|(32,3)),
            "geometry orientation/frame lifecycle is pending: mask {}, frame {}",params.mask,params.index
        );
        Ok(Self {
            params,
            stage: Stage::Entry,
            rectangle: None,
            margins: [0; 4],
            clipped: None,
            plan: None,
            next: 0,
        })
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn rectangle(&self) -> Result<[i32; 4]> {
        self.rectangle.context("geometry output rectangle absent")
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (matches!(self.params.mask, 1 | 2) && self.params.index <= 1
                || matches!((self.params.mask,self.params.index),(16,2)|(32,3)))
                && self.next <= self.plan.as_ref().map_or(0, |p| p.writes.len()),
            "geometry renderer job invalid"
        );
        Ok(())
    }
    pub fn at_cut(&self, pc: u32) -> bool {
        matches!(
            (pc, self.stage),
            (0x4c79ca, Stage::Entry)
                | (0x4c8e0d, Stage::ResourceFirst)
                | (0x4c7a0e, Stage::FirstReturned)
                | (0x4c8e0d, Stage::ResourceSecond)
                | (0x4c7a1a, Stage::SecondReturned)
                | (0x4c19b1, Stage::FrameFirst)
                | (0x4c7a41, Stage::FrameFirstReturned)
                | (0x4c19b1, Stage::FrameSecond)
                | (0x4c7a4d, Stage::FrameSecondReturned)
                | (0x4c7fdb, Stage::Setup)
                | (0x4c76d1, Stage::Setup)
                | (0x4c7a79, Stage::SetupReturned)
                | (0x4c77bc, Stage::Clip)
                | (0x4c7ae9, Stage::ClipReturned)
                | (0x4c7b98, Stage::RectReturned)
                | (0x4c7f75, Stage::BlitCall)
                | (0x4c4907, Stage::Blit)
                | (0x4c7f7a, Stage::BlitReturned)
                | (0x4c78c5, Stage::Shade)
                | (0x4b954a, Stage::Palette)
                | (0x4c77a1, Stage::PaletteReturned)
        )
    }
    pub(super) fn advance(&mut self, s: &mut Source, d: &mut Data, c: &Frame) -> Result<bool> {
        self.validate()?;
        s.validate()?;
        ensure!(
            d.color_pitch == s.targets.color_pitch as i32
                && d.depth_pitch == s.targets.depth_pitch as i32,
            "geometry shared plane pitches differ"
        );
        self.stage = match self.stage {
            Stage::Entry => Stage::ResourceFirst,
            Stage::ResourceFirst => {
                s.sprite(
                    self.params.mask_cache(),
                    self.params.coverage_image,
                    self.params.index,
                )?;
                Stage::FirstReturned
            }
            Stage::FirstReturned => Stage::ResourceSecond,
            Stage::ResourceSecond => {
                s.pair(&self.params)?;
                Stage::SecondReturned
            }
            Stage::SecondReturned => Stage::FrameFirst,
            Stage::FrameFirst => Stage::FrameFirstReturned,
            Stage::FrameFirstReturned => Stage::FrameSecond,
            Stage::FrameSecond => Stage::FrameSecondReturned,
            Stage::FrameSecondReturned => Stage::Setup,
            Stage::Setup => {
                let (mask, color) = s.pair(&self.params)?;
                ensure!(
                    mask.encoded[..8] == color.encoded[..8],
                    "geometry unequal frame origins need C4 adjustment"
                );
                let w = c.state().words();
                let packed = w.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
                ensure!(
                    packed[0x35..0x39] == [0; 4],
                    "geometry palette modification is pending"
                );
                let h = &mask.encoded;
                let [x, y] = self.params.point;
                self.rectangle = Some([
                    i32::from(short(h, 2).wrapping_add(x as i16)),
                    i32::from(short(h, 0).wrapping_add(y as i16)),
                    i32::from(short(h, 6).wrapping_add(x as i16)),
                    i32::from(short(h, 4).wrapping_add(y as i16)),
                ]);
                if s.palette_owner(color)?.palette_selection.is_some() {
                    Stage::Palette
                } else {
                    s.selected_palette(color, c)?;
                    Stage::SetupReturned
                }
            }
            Stage::Palette => {
                let (_, color) = s.pair(&self.params)?;
                s.selected_palette(color, c)?;
                Stage::PaletteReturned
            }
            Stage::PaletteReturned => Stage::SetupReturned,
            Stage::SetupReturned => Stage::Clip,
            Stage::Clip => {
                let r = self.rectangle()?;
                let k = self.params.clip;
                self.clipped = Some(!(r[0] < k[2] && r[1] < k[3] && k[0] < r[2] && k[1] < r[3]));
                if self.clipped == Some(false) {
                    self.margins = [
                        (k[0] - r[0]).max(0),
                        (k[1] - r[1]).max(0),
                        (r[2] - k[2]).max(0),
                        (r[3] - k[3]).max(0),
                    ];
                }
                Stage::ClipReturned
            }
            Stage::ClipReturned => {
                if self.clipped == Some(true)
                    || c.state().flags(0).map_err(anyhow::Error::msg)? & 3 != 3
                {
                    Stage::Complete
                } else {
                    Stage::Rect
                }
            }
            Stage::Rect => Stage::RectReturned,
            Stage::RectReturned => {
                if s.shading {
                    Stage::Shade
                } else {
                    Stage::BlitCall
                }
            }
            Stage::Shade => {
                ensure!(
                    matches!(self.params.mask,16|32)
                        || self.params.descriptor[0] == 127 && self.params.descriptor[2] == 127,
                    "geometry gradient shading services are pending"
                );
                //4c78c5 returns zero for either diagonal wall mask before
                //reading descriptor pairs or writing its output locals.
                Stage::BlitCall
            }
            Stage::BlitCall => {
                self.plan = Some(s.plan(self, c, d)?);
                Stage::Blit
            }
            Stage::Blit => Stage::Drawing,
            Stage::Drawing => {
                let p = self.plan.as_ref().context("geometry write queue absent")?;
                if let Some(w) = p.writes.get(self.next) {
                    sprite_full::apply(w, &mut d.color, &mut d.depth)
                        .map_err(anyhow::Error::msg)?;
                    self.next += 1;
                    Stage::Drawing
                } else {
                    s.mask_scratch[..p.mask_scratch.len()].copy_from_slice(&p.mask_scratch);
                    s.color_scratch[..p.color_scratch.len()].copy_from_slice(&p.color_scratch);
                    Stage::BlitReturned
                }
            }
            Stage::BlitReturned => Stage::Complete,
            Stage::Complete => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
    pub(super) fn compare(&self, q: &serde_json::Value, s: &Source, c: &Frame) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("geometry renderer PC absent")? as u32;
        ensure!(self.at_cut(pc), "geometry renderer stage differs");
        let e = &q["visible_geometry"];
        let stack = crate::original_session::completed_animations::bytes(
            e["stack"]["raw"]
                .as_str()
                .context("renderer stack absent")?,
        )?;
        let (mask, color) = s.pair(&self.params)?;
        match self.stage {
            Stage::Entry => ensure!(
                q["registers"]["ecx"] == self.params.coverage_cache
                    && word(&stack, 8) == self.params.coverage_override
                    && word(&stack, 12) == self.params.coverage_image
                    && word(&stack, 16) == self.params.wallpaper_cache
                    && word(&stack, 20) == self.params.wallpaper_image
                    && word(&stack, 24) == self.params.index,
                "C4 entry resources differ"
            ),
            Stage::ResourceFirst | Stage::ResourceSecond => {
                let r = if self.stage == Stage::ResourceFirst {
                    mask
                } else {
                    color
                };
                ensure!(
                    q["registers"]["ecx"] == r.cache && word(&stack, 4) == r.image,
                    "C4 sprite query differs"
                );
            }
            Stage::FirstReturned => ensure!(
                q["registers"]["eax"] == mask.resource,
                "C4 first resource result differs"
            ),
            Stage::SecondReturned => ensure!(
                q["registers"]["eax"] == color.resource,
                "C4 second resource result differs"
            ),
            Stage::Palette => {
                let owner = s
                    .palette_owner(color)?
                    .palette_selection
                    .as_ref()
                    .context("palette selector absent")?;
                ensure!(
                    q["registers"]["ecx"] == owner.owner
                        && word(&stack, 4) == c.state().words()[18],
                    "computed palette selection call differs"
                );
                s.compare_palette(q, color)?;
            }
            Stage::PaletteReturned => {
                ensure!(
                    q["registers"]["eax"] == s.selected_palette(color, c)?.0,
                    "computed palette selection result differs"
                );
                s.compare_palette(q, color)?;
            }
            Stage::FrameFirst | Stage::FrameSecond => {
                let r = if self.stage == Stage::FrameFirst {
                    mask
                } else {
                    color
                };
                ensure!(
                    q["registers"]["ecx"] == r.resource && word(&stack, 4) == self.params.index,
                    "C4 frame getter differs"
                );
            }
            Stage::FrameFirstReturned => ensure!(
                q["registers"]["eax"] == mask.frame,
                "C4 first frame differs"
            ),
            Stage::FrameSecondReturned => ensure!(
                q["registers"]["eax"] == color.frame,
                "C4 second frame differs"
            ),
            Stage::ClipReturned => ensure!(
                q["registers"]["eax"].as_u64().context("clip AL absent")? & 255
                    == u64::from(self.clipped.unwrap()),
                "C4 clip result differs"
            ),
            Stage::Setup => {
                let bp = e["renderer_frame"]["bp"].as_u64().context("C4 BP absent")? as u32;
                ensure!(
                    q["registers"]["ecx"] == bp - 0xbc,
                    "C4 setup receiver differs"
                );
                let context = q["context"]["pointer"]
                    .as_u64()
                    .context("context pointer absent")? as u32;
                let expected = if pc == 0x4c7fdb {
                    vec![
                        mask.frame + 10,
                        color.frame + 10,
                        word(&color.raw, 8),
                        word(&color.raw, 0x20),
                        mask.frame,
                        color.frame,
                        self.params.point[0] as u32,
                        self.params.point[1] as u32,
                        context,
                        0,
                        self.params.offset as u32,
                    ]
                } else {
                    vec![
                        color.frame + 10,
                        mask.frame + 10,
                        word(&color.raw, 8),
                        word(&color.raw, 0x20),
                        mask.frame,
                        self.params.point[0] as u32,
                        self.params.point[1] as u32,
                        context,
                        0,
                        1,
                    ]
                };
                ensure!(
                    expected.iter().enumerate().all(|(i, v)| {
                        let actual = word(&stack, 4 + 4 * i);
                        //4c7fde writes AX, preserving EAX's upper half;
                        //4c76d4 consumes only the argument's low16 bits.
                        if pc == 0x4c76d1 && i == 8 {
                            actual & 0xffff == *v
                        } else {
                            actual == *v
                        }
                    }),
                    "computed C4 setup arguments differ"
                );
            }
            Stage::Shade => {
                let bp = q["child_frame"]["bp"].as_u64().context("child BP absent")? as u32;
                ensure!(
                    word(&stack, 4) == bp - 0x54
                        && word(&stack, 8) == self.params.mask
                        && word(&stack, 12) == c.state().words()[8],
                    "C4 shading arguments differ"
                );
                let parent = crate::original_session::completed_animations::bytes(
                    q["geometry_prefix"]["frame"]["raw"]
                        .as_str()
                        .context("child frame absent")?,
                )?;
                ensure!(
                    parent[0x178 - 0x54..0x178 - 0x50] == self.params.descriptor,
                    "C4 shading descriptor differs"
                );
            }
            _ => (),
        }
        if matches!(
            self.stage,
            Stage::SetupReturned
                | Stage::Clip
                | Stage::ClipReturned
                | Stage::RectReturned
                | Stage::BlitCall
                | Stage::Blit
                | Stage::BlitReturned
        ) {
            let f = crate::original_session::completed_animations::bytes(
                e["renderer_frame"]["raw"]
                    .as_str()
                    .context("renderer locals absent")?,
            )?;
            let rect = self.rectangle()?;
            let packed = [
                rect[1] as i16,
                rect[0] as i16,
                rect[3] as i16,
                rect[2] as i16,
            ]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
            ensure!(
                word(&f, 0x34) == color.frame + 10
                    && word(&f, 0x38) == mask.frame + 10
                    && f[0x3c..0x44] == packed
                    && word(&f, 0x34 + 0x2c) == word(&color.raw, 8)
                    && word(&f, 0x34 + 0x30) == s.selected_palette(color, c)?.0,
                "C4 setup fields differ"
            );
            if matches!(
                self.stage,
                Stage::ClipReturned
                    | Stage::RectReturned
                    | Stage::BlitCall
                    | Stage::Blit
                    | Stage::BlitReturned
            ) {
                ensure!(
                    [
                        word(&f, 0x34 + 0x1c) as i32,
                        word(&f, 0x34 + 0x24) as i32,
                        word(&f, 0x34 + 0x20) as i32,
                        word(&f, 0x34 + 0x28) as i32
                    ] == self.margins,
                    "C4 clipping margins differ"
                );
            }
            if matches!(
                self.stage,
                Stage::RectReturned | Stage::BlitCall | Stage::Blit | Stage::BlitReturned
            ) {
                ensure!(
                    std::array::from_fn::<_, 4, _>(|i| word(&f, 12 + 4 * i) as i32) == self.margins,
                    "C4 SetRect publication differs"
                );
            }
            if self.stage == Stage::Blit {
                let bp = e["renderer_frame"]["bp"].as_u64().context("C4 BP absent")? as u32;
                let w = c.state().words();
                let z = if matches!(self.params.mask,16|32) { word(&s.depth,0x38) } else {
                    word(&s.depth, (14 - w[8]) as usize * 4)
                    + (16u32.wrapping_shl(w[8])) * 2
                    + (self.params.offset as u32) * 2 };
                let expected = [
                    0x4c7f7a,
                    rect[0] as u32,
                    rect[1] as u32,
                    (rect[2] - rect[0]) as u32,
                    (rect[3] - rect[1]) as u32,
                    bp - 0x44,
                    bp - 0xd4,
                    s.targets.color
                        + ((rect[1] as u32) * (s.targets.color_pitch / 2) + rect[0] as u32) * 2,
                    s.targets.color_pitch / 2,
                    s.targets.depth
                        + ((rect[1] as u32) * (s.targets.depth_pitch / 2) + rect[0] as u32) * 2,
                    s.targets.depth_pitch / 2,
                    bp - 0xe4,
                    z,
                    q["context"]["pointer"].as_u64().unwrap() as u32,
                    if self.params.mask == 32 { (-300i32) as u32 } else { 0 },
                ];
                ensure!(
                    expected
                        .iter()
                        .enumerate()
                        .all(|(i, v)| word(&stack, i * 4) == *v),
                    "computed ordinary blitter arguments differ"
                );
            }
        }
        s.compare(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn unit_palette_bypasses_stale_generation_but_adjusted_palette_does_not() {
        let mut header = vec![0; 32];
        header[0x18..0x1c].copy_from_slice(&0.5f32.to_bits().to_le_bytes());
        let mut p = Palette {
            owner: 0x1000,
            header,
            adjusted: vec![0; 512],
            generation: 7,
            unit_light: 1f32.to_bits(),
            epsilon: 0.000001f32.to_bits(),
        };
        assert_eq!(p.select(1f32.to_bits()).unwrap(), 0x1420);
        p.header[0x18..0x1c].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert_eq!(p.select(1f32.to_bits()).unwrap(), 0x1420);
        p.header[0x18..0x1c].copy_from_slice(&0.5f32.to_bits().to_le_bytes());
        assert!(p
            .select(0.5f32.to_bits())
            .unwrap_err()
            .to_string()
            .contains("generation refresh"));
        p.header[0x1c..0x20].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(p.select(0.5f32.to_bits()).unwrap(), 0x1c20);
        assert!(p
            .select(0.75f32.to_bits())
            .unwrap_err()
            .to_string()
            .contains("brightness update"));
    }
}

impl Source {
    ///Import B residency without replacing hot scratch rows or target planes.
    pub(super) fn extend_catalog(&mut self, c: &super::geometry_catalog::Catalog) -> Result<()> {
        c.validate()?;
        let zoom = word(&c.read(0x6f00d0, 16)?, 8);
        let depth = c.read(self.depth_owner, 0xb0)?;
        let curve = c
            .read(word(&depth, 4 + (3 - zoom) as usize * 4) - 256, 2048)?
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        let columns = c
            .read(word(&depth, (14 - zoom) as usize * 4) - 256, 1024)?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        ensure!(
            self.depth == depth
                && self.zoom == zoom
                && self.curve == curve
                && self.columns == columns
                && self.shading == (c.read(0x6f018a, 1)?[0] != 0),
            "catalog depth/shading aliases differ"
        );
        let mut groups: BTreeMap<u32, Vec<Sprite>> = BTreeMap::new();
        for r in &c.resources {
            for (index, (frame, body)) in &r.frames {
                if let Some(old) = self.find_sprite(r.cache, r.image, *index) {
                    ensure!(
                        old.frame == *frame && old.encoded == *body && old.raw == r.header,
                        "catalog enrichment changes a retained sprite"
                    );
                    continue;
                }

                groups.entry(word(&r.header, 8)).or_default().push(Sprite {
                    index: *index,
                    cache: r.cache,
                    image: r.image,
                    resource: r.pointer,
                    directory: word(&r.header, 12),
                    frame: *frame,
                    raw: r.header.clone(),
                    encoded: body.clone(),
                });
            }
        }
        let mut sources = vec![];
        for (owner, sprites) in groups {
            let raw = c.read(owner, 0x1000)?;
            let s = Self {
                source: c.source().into(),
                sprites,
                palette: raw[0x420..0x620].to_vec(),
                palette_address: owner + 0x420,
                depth_owner: self.depth_owner,
                depth: depth.clone(),
                curve: curve.clone(),
                columns: columns.clone(),
                diagonal_columns: self.diagonal_columns.clone(),
                zoom,
                targets: self.targets,
                mask_scratch: self.mask_scratch.clone(),
                color_scratch: self.color_scratch.clone(),
                shading: self.shading,
                palette_selection: Some(Palette {
                    owner,
                    header: raw[..32].to_vec(),
                    adjusted: raw[0xc20..0xe20].to_vec(),
                    generation: word(&c.read(0x6f0184, 4)?, 0),
                    unit_light: word(&c.read(0x666748, 4)?, 0),
                    epsilon: word(&c.read(0x67343c, 4)?, 0),
                }),
                extra: None,
            };
            s.validate()?;
            sources.push(s);
        }
        let mut tail = self;
        while let Some(ref mut extra) = tail.extra {
            tail = extra;
        }
        for s in sources {
            tail.extra = Some(Box::new(s));
            tail = tail.extra.as_mut().unwrap();
        }
        Ok(())
    }
}
