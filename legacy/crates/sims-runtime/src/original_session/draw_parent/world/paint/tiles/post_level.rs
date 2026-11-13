//!4ad2d9..4ad368: the retained region-list caller, followed by the existing
//!4ad4bd scanner. Clip writes remain ordered; SETL changes AL, not all of EAX.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Source,
    First,
    AfterFirst,
    Second,
    Copy,
    Copied,
    Call,
    Scan,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct State {
    stage: Stage,
    rectangle: [i32; 4],
    region: u32,
    level: i32,
    cached_level: i32,
    step: [i8; 3],
    frame: Option<Frame>,
}
impl State {
    pub fn new(
        rectangle: [i32; 4],
        region: u32,
        level: i32,
        cached_level: i32,
        step: [i8; 3],
    ) -> Self {
        Self {
            stage: Stage::Entry,
            rectangle,
            region,
            level,
            cached_level,
            step,
            frame: None,
        }
    }
    fn clip(&self) -> [i16; 4] {
        [
            self.rectangle[1] as i16,
            self.rectangle[0] as i16,
            self.rectangle[3] as i16,
            self.rectangle[2] as i16,
        ]
    }
    fn packed(&self, half: usize) -> u32 {
        let c = self.clip();
        u32::from(c[half * 2] as u16) | (u32::from(c[half * 2 + 1] as u16) << 16)
    }
    fn above(&self) -> u32 {
        (self.packed(1) & !0xff) | u32::from(self.level < self.cached_level)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.region != 0 && self.level >= 1,
            "post-level region/level absent"
        );
        ensure!(
            self.frame.is_some() == matches!(self.stage, Stage::Scan | Stage::Returned),
            "post-level scan lifetime differs"
        );
        if let Some(f) = &self.frame {
            f.validate().map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub fn second(&self)->Option<bool>{self.frame.as_ref().map(Frame::second)}
    pub fn row(&self) -> Option<&tile_scan::Request> {
        self.frame.as_ref().and_then(Frame::pending)
    }
    pub fn return_row(&mut self, r: &tile_scan::Request) -> Result<()> {
        self.frame
            .as_mut()
            .context("post-level frame absent")?
            .acknowledge(r, r.point, r.step)
            .map_err(anyhow::Error::msg)
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    pub fn advance(&mut self, inputs: Inputs<'_>, surfaces: &mut surfaces::State) -> Result<bool> {
        self.validate()?;
        self.stage = match self.stage {
            Stage::Entry => Stage::Source,
            Stage::Source => Stage::First,
            Stage::First => {
                let c = self.clip();
                surfaces.set_window_clip_half(0, [c[0], c[1]])?;
                Stage::AfterFirst
            }
            Stage::AfterFirst => Stage::Second,
            Stage::Second => {
                let c = self.clip();
                surfaces.set_window_clip_half(1, [c[2], c[3]])?;
                Stage::Copy
            }
            Stage::Copy => Stage::Copied,
            Stage::Copied => Stage::Call,
            Stage::Call => {
                self.frame = Some(Frame::new(self.level, self.above(), 0, self.step));
                Stage::Scan
            }
            Stage::Scan => {
                let f = self.frame.as_mut().context("post-level frame absent")?;
                if !f.advance(inputs).map_err(anyhow::Error::msg)? {
                    return Ok(false);
                }
                if f.complete() {
                    Stage::Returned
                } else {
                    Stage::Scan
                }
            }
            Stage::Returned => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4ad2d9 => self.stage == Stage::Entry,
            0x4ad2df => self.stage == Stage::Source,
            0x4ad304 => self.stage == Stage::First,
            0x4ad309 => self.stage == Stage::AfterFirst,
            0x4ad30c => self.stage == Stage::Second,
            0x4ad345 => self.stage == Stage::Copy,
            0x4ad34f => self.stage == Stage::Copied,
            0x4ad363 => self.stage == Stage::Call,
            0x4ad368 => self.stage == Stage::Returned,
            _ => {
                self.stage == Stage::Scan
                    && self.frame.as_ref().is_some_and(|f| f.at_native_cut(pc))
            }
        }
    }
    pub fn compare(&self, q: &serde_json::Value, surfaces: &surfaces::State) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("post-level PC absent")? as u32;
        ensure!(self.at_native_cut(pc), "post-level boundary differs");
        let p = &q["post_base"];
        let bp = p["parent_bp"].as_u64().context("post-level BP absent")? as u32;
        let raw = decode(&p["parent_frame"]["raw"])?;
        ensure!(
            p["parent_frame"]["address"] == bp - 0x70 && raw.len() == 0xa0,
            "post-level parent extent differs"
        );
        // This model represents the NULL explicit-clip, region-list caller.
        // A different ABI is an unsupported caller, never a guessed reply.
        ensure!(
            word(&raw, 0x7c) == 0,
            "post-level explicit clip caller unsupported"
        );
        ensure!(
            word(&raw, 0x48) as i32 == self.cached_level
                && word(&raw, 0x44) as i32 == self.level
                && word(&raw, 0x58) == self.region,
            "post-level parent owners differ"
        );
        ensure!(
            decode(&p["clip"])?
                == surfaces
                    .window_clip()
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            "post-level shared clip differs"
        );
        if matches!(
            self.stage,
            Stage::First
                | Stage::AfterFirst
                | Stage::Second
                | Stage::Copy
                | Stage::Copied
                | Stage::Call
                | Stage::Scan
                | Stage::Returned
        ) {
            ensure!(
                word(&raw, 0x38) == self.packed(0) && word(&raw, 0x3c) == self.packed(1),
                "post-level packed locals differ"
            );
        }
        if matches!(
            self.stage,
            Stage::Copied | Stage::Call | Stage::Scan | Stage::Returned
        ) {
            ensure!(
                std::array::from_fn::<_, 4, _>(|i| word(&raw, 0x20 + 4 * i) as i32)
                    == self.rectangle,
                "post-level copied rectangle differs"
            );
        }
        let eax = q["registers"]["eax"]
            .as_u64()
            .context("post-level EAX absent")? as u32;
        if matches!(self.stage, Stage::First | Stage::AfterFirst) {
            ensure!(
                eax == self.packed(0),
                "post-level first clip register differs"
            );
        }
        if matches!(self.stage, Stage::Second | Stage::Copy | Stage::Copied) {
            ensure!(
                eax == self.packed(1),
                "post-level second clip register differs"
            );
        }
        if self.stage == Stage::Call {
            let stack = decode(&p["stack"]["raw"])?;
            ensure!(
                eax == self.above()
                    && std::array::from_fn::<_, 4, _>(|i| word(&stack, 4 * i))
                        == [0x6f0c10, self.level as u32, self.above(), 0]
                    && word(&stack, 16)
                        == q["context"]["pointer"]
                            .as_u64()
                            .context("post-level context absent")?
                            as u32,
                "post-level call ABI differs"
            );
        }
        if self.stage == Stage::Scan {
            let f = self.frame.as_ref().unwrap();
            let frame = &q["scan_frame"];
            if pc == 0x4ad4bd {
                let stack = decode(&p["stack"]["raw"])?;
                ensure!(
                    std::array::from_fn::<_, 5, _>(|i| word(&stack, 4 * i))
                        == [0x4ad368, 0x6f0c10, self.level as u32, self.above(), 0],
                    "post-level scanner entry ABI differs"
                );
            } else {
                let raw = decode(&frame["raw"])?;
                let bp = frame["bp"].as_u64().context("post-level scan BP absent")? as u32;
                ensure!(
                    frame["address"] == bp - 0x38 && raw.len() == 96,
                    "post-level scanner extent differs"
                );
                ensure!(
                    raw[0x18..0x1b] == f.step().map(|v| v as u8),
                    "post-level copied step differs"
                );
                if pc != 0x4ad4df {
                    ensure!(
                        word(&raw, 0x10) as i32 == f.index()
                            && raw[0x20..0x23] == f.point().map(|v| v as u8),
                        "post-level scan point/index differ"
                    );
                    if pc != 0x4ad548 {
                        ensure!(
                            [word(&raw, 0x1c) as i32, word(&raw, 0x14) as i32] == f.screen(),
                            "post-level scan projection differs"
                        );
                    }
                }
                if let Some(r) = f.pending() {
                    let stack = decode(&p["stack"]["raw"])?;
                    ensure!(
                        word(&stack, 0) == if f.second(){0x4ad885}else{0x4ad6ab}
                            && word(&stack, 4) == bp - 0x18
                            && word(&stack, 8) as i32 == r.count
                            && word(&stack, 12) == bp - 0x20
                            && [word(&stack, 16) as i32, word(&stack, 20) as i32] == r.screen,
                        "post-level row ABI differs"
                    );
                }
            }
        }
        Ok(())
    }
}
