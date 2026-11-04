//! 4c8383..4c848a: retained classifier, conversion, mask and one-gradient draw.
//! Later observations are comparison data, never child replies.
use super::*;
use sims_sim::lot_presenter::{sprite_full as raster, terrain_raster::Data};

pub(in super::super) struct Draw<'a> {
    pub frame: &'a [u8],
    pub frame_address: u32,
    pub image: u32,
    pub origin: [i32; 2],
    pub base: i32,
    pub rows: &'a [i32],
    pub margins: Option<[i32; 4]>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Classify,
    Classified,
    Convert,
    Converted,
    Kernel,
    Address,
    Mask,
    Dispatch,
    Blit,
    Writing,
    Returned,
    GradientDone,
    FlatDone,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in super::super) struct Cursor {
    stage: Stage,
    descriptor: [u8; 4],
    radius: i32,
    zoom: u8,
    brightness: u32,
    count: Option<u32>,
    // A native classifier leaves some caller stack words untouched.
    // Preserve that uncertainty rather than manufacturing zero-valued locals.
    outputs: [Option<i32>; 4],
    intensity: Option<i32>,
    mask: Option<(u32, i32)>,
    writes: Vec<raster::Write>,
    next: usize,
}
impl Cursor {
    pub fn new(descriptor: [u8; 4], radius: i32, zoom: u8, brightness: u32) -> Self {
        Self {
            stage: Stage::Classify,
            descriptor,
            radius,
            zoom,
            brightness,
            count: None,
            outputs: [None; 4],
            intensity: None,
            mask: None,
            writes: vec![],
            next: 0,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.zoom <= 3 && self.radius >= 0 && self.next <= self.writes.len(),
            "gradient child extent differs"
        );
        ensure!(
            self.count.is_some() == (self.stage != Stage::Classify),
            "gradient classifier result lifetime differs"
        );
        if !matches!(
            self.stage,
            Stage::Classify | Stage::Classified | Stage::Convert | Stage::FlatDone
        ) {
            ensure!(
                self.intensity.is_some()
                    && self.count == Some(1)
                    && self.outputs[..2].iter().all(Option::is_some),
                "single-gradient suspended inputs absent"
            );
        }
        Ok(())
    }
    pub fn complete(&self) -> bool {
        matches!(self.stage, Stage::FlatDone | Stage::GradientDone)
    }
    pub fn gradient_done(&self) -> bool {
        self.stage == Stage::GradientDone
    }
    pub fn pc(&self, clipped: bool) -> Option<u32> {
        Some(match self.stage {
            Stage::Classify => 0x4c8049,
            Stage::Classified => 0x4c8388,
            Stage::Convert => 0x4c845c,
            Stage::Converted => 0x4c8461,
            Stage::Kernel => 0x4b5c52,
            Stage::Address => 0x4a3813,
            Stage::Mask => 0x4c847b,
            Stage::Dispatch => 0x4b6b9f,
            Stage::Blit => {
                if clipped {
                    0x4b71b5
                } else {
                    0x4b6dad
                }
            }
            Stage::Returned => 0x4c848a,
            Stage::Writing | Stage::GradientDone | Stage::FlatDone => return None,
        })
    }
    pub fn advance(&mut self, source: &Source, input: Draw<'_>, data: &mut Data) -> Result<bool> {
        self.validate()?;
        source.validate()?;
        self.stage = match self.stage {
            Stage::Classify => {
                let (count, _, writes) = sims_sim::lot_presenter::floor_light::classify(
                    self.descriptor.map(|v| v as i8),
                    self.radius,
                    [0; 4],
                );
                for w in writes {
                    self.outputs[w.offset / 4] = Some(w.value as i32);
                }
                self.count = Some(count);
                Stage::Classified
            }
            Stage::Classified => {
                if !source.needs_gradient(self.count.unwrap(), self.brightness) {
                    Stage::FlatDone
                } else {
                    ensure!(
                        self.count == Some(1),
                        "two-gradient sprite lighting remains pending"
                    );
                    Stage::Convert
                }
            }
            Stage::Convert => {
                // The qualified original render path uses FCW=003f. This is
                // the same admitted control as placement/lighting arithmetic.
                self.intensity = Some(
                    sims_sim::lot_presenter::sprite_gradient::intensity(self.brightness, 0x003f)
                        .map_err(anyhow::Error::msg)?,
                );
                Stage::Converted
            }
            Stage::Converted => Stage::Kernel,
            Stage::Kernel => {
                ensure!(
                    self.radius == source.radius(),
                    "gradient zoom/bitmap owner differs"
                );
                Stage::Address
            }
            Stage::Address => {
                let (p, pitch, _) =
                    source.mask(self.outputs[0].unwrap(), self.outputs[1].unwrap())?;
                self.mask = Some((p, pitch));
                Stage::Mask
            }
            Stage::Mask => Stage::Dispatch,
            Stage::Dispatch => Stage::Blit,
            Stage::Blit => {
                let (_, palette) = source.palette(input.image)?;
                let (_, pitch, mask) =
                    source.mask(self.outputs[0].unwrap(), self.outputs[1].unwrap())?;
                let sprite = raster::Input {
                    frame: input.frame,
                    palette,
                    rows: input.rows,
                    base: input.base,
                    x: input.origin[0],
                    y: input.origin[1],
                    color_pitch: data.color_pitch / 2,
                    depth_pitch: data.depth_pitch / 2,
                };
                self.writes = sims_sim::lot_presenter::sprite_gradient::plan(
                    sims_sim::lot_presenter::sprite_gradient::Input {
                        sprite,
                        mask,
                        mask_pitch: pitch,
                        light: self.intensity.unwrap(),
                        tables: source.tables(),
                    },
                    input.margins,
                )
                .map_err(anyhow::Error::msg)?;
                Stage::Writing
            }
            Stage::Writing => {
                if let Some(w) = self.writes.get(self.next) {
                    raster::apply(w, &mut data.color, &mut data.depth)
                        .map_err(anyhow::Error::msg)?;
                    self.next += 1;
                    Stage::Writing
                } else {
                    Stage::Returned
                }
            }
            Stage::Returned => Stage::GradientDone,
            Stage::FlatDone | Stage::GradientDone => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
    pub fn compare(
        &self,
        source: &Source,
        input: Draw<'_>,
        q: &serde_json::Value,
        data: &Data,
    ) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("gradient PC absent")? as u32;
        ensure!(
            self.pc(input.margins.is_some()) == Some(pc),
            "gradient child boundary differs"
        );
        let x = &q["gradient"];
        let args = x["arguments"]
            .as_array()
            .context("gradient arguments absent")?;
        let a = |i: usize| -> Result<u32> {
            Ok(args
                .get(i)
                .and_then(|v| v.as_u64())
                .context("gradient argument absent")? as u32)
        };
        let bytes = |v: &serde_json::Value| -> Result<Vec<u8>> {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("gradient bytes absent")?,
            )
        };
        ensure!(
            bytes(&x["kernel_owner"])? == source.owner && bytes(&x["flags"])? == source.flags,
            "gradient live input owner differs"
        );
        if pc == 0x4c8049 {
            ensure!(
                bytes(&x["descriptor"])? == self.descriptor && a(2)? == u32::from(self.zoom),
                "gradient classifier input differs"
            );
        }
        if pc == 0x4c8388 {
            ensure!(
                x["count"] == self.count.unwrap() && q["registers"]["eax"] == self.count.unwrap(),
                "gradient classifier result differs"
            );
        }
        if pc == 0x4c8461 {
            ensure!(
                x["intensity"] == serde_json::json!([self.intensity.unwrap(), 0]),
                "gradient intensity conversion differs"
            );
        }
        if matches!(pc, 0x4b5c52 | 0x4a3813) {
            let i = if pc == 0x4b5c52 { 2 } else { 1 };
            ensure!(
                a(i)? as i32 == self.outputs[0].unwrap()
                    && a(i + 1)? as i32 == self.outputs[1].unwrap(),
                "gradient mask coordinates differ"
            );
            let (p, header) = source.header();
            ensure!(
                x["header_pointer"] == p && bytes(&x["header"])? == header,
                "gradient bitmap owner differs"
            );
            if pc == 0x4b5c52 {
                ensure!(a(4)? == u32::from(self.zoom), "gradient mask zoom differs");
            }
        }
        if pc == 0x4c847b {
            let raw = bytes(&x["output"])?;
            let (p, pitch, mask) =
                source.mask(self.outputs[0].unwrap(), self.outputs[1].unwrap())?;
            ensure!(
                raw.len() == 8 && [word(&raw, 0), word(&raw, 4)] == [p, pitch as u32],
                "gradient returned mask differs"
            );
            for (n, r) in x["mask_rows"]
                .as_array()
                .context("gradient mask rows absent")?
                .iter()
                .enumerate()
            {
                let b = bytes(&r["raw"])?;
                ensure!(
                    r["pointer"] == p + n as u32 * pitch as u32
                        && mask.get(n * pitch as usize..n * pitch as usize + b.len())
                            == Some(b.as_slice()),
                    "gradient mask row differs"
                );
            }
        }
        if matches!(pc, 0x4b6b9f | 0x4b6dad | 0x4b71b5) {
            let color = bytes(&x["color_descriptor"])?;
            let depth = bytes(&x["depth_descriptor"])?;
            ensure!(
                color.len() == 8
                    && depth.len() == 8
                    && word(&color, 4) == (data.color_pitch / 2) as u32
                    && word(&depth, 4) == (data.depth_pitch / 2) as u32,
                "gradient destination pitches differ"
            );
            ensure!(
                a(3)? == input.frame_address
                    && bytes(&x["sprite_header"])? == input.frame[..16]
                    && a(5)? as i32 == self.intensity.unwrap()
                    && a(7)? as i32 == input.base,
                "gradient sprite/depth/intensity arguments differ"
            );
            let (palette, pbytes) = source.palette(input.image)?;
            ensure!(
                a(6)? == palette && bytes(&x["raw_palette"])? == pbytes,
                "gradient raw palette differs"
            );
            let m = bytes(&x["mask_descriptor"])?;
            ensure!(
                m.len() == 8
                    && [word(&m, 0), word(&m, 4)] == {
                        let (p, pitch, _) =
                            source.mask(self.outputs[0].unwrap(), self.outputs[1].unwrap())?;
                        [p, pitch as u32]
                    },
                "gradient dispatch mask differs"
            );
            let rows = bytes(&x["depth_rows"])?;
            ensure!(
                input.rows.len() * 4 >= rows.len()
                    && input
                        .rows
                        .iter()
                        .take(rows.len() / 4)
                        .enumerate()
                        .all(|(i, v)| word(&rows, i * 4) as i32 == *v),
                "gradient depth rows differ"
            );
            let raw = bytes(&x["clip"])?;
            ensure!(
                raw.len() == 16
                    && std::array::from_fn::<_, 4, _>(|i| word(&raw, i * 4) as i32)
                        == input.margins.unwrap_or([0; 4]),
                "gradient clipping margins differ"
            );
        }
        Ok(())
    }
}
