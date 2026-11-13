//!4a8b1e..4a8bad: cached camera guard and shared motion-frame cursor.
use super::*;
use anyhow::Context as _;
use sims_sim::lot_presenter::{centering, motion::frame};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Cache,
    Frame,
    Waiting,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Construct { renderer: u32, zoom: u32 },
    Motion(frame::Request),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    owner: u32,
    renderer: u32,
    valid: [u8; 3],
    motion: centering::State,
    stage: Stage,
    frame: Option<frame::Cursor>,
}
impl State {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        ensure!(
            source == "c92c35dd68417da337a4ac34b042c6e5c77105f257d412e5ede8093a63c04d1e",
            "camera preparation source differs"
        );
        let value: serde_json::Value = serde_json::from_slice(encoded)?;
        let m = Memory::new(&value)?;
        let owner = value["classification"]["view"]
            .as_u64()
            .context("camera preparation view absent")? as u32;
        let vr = m.read(owner, 0x4c0)?;
        let lot = m.read(
            value["lot_world"].as_u64().context("camera lot absent")? as u32,
            0x88,
        )?;
        let coords = m.read(0x6f00d0, 16)?;
        let motion = centering::State {
            level: word(&vr, 0x18) as i32,
            origin: [word(&coords, 0) as i32, word(&coords, 4) as i32],
            pending: [word(&vr, 0x170) as i32, word(&vr, 0x174) as i32],
            viewport: std::array::from_fn(|i| word(&vr, 0x28 + 4 * i) as i32),
            bounds: std::array::from_fn(|i| word(&lot, 0x70 + 4 * i) as i32),
            zoom: word(&coords, 8),
            graphics_ready: word(&vr, 0x68),
            motion_active: word(&vr, 0x1a4),
            motion_policy: word(&vr, 0x1a8),
            motion_duration: word(&vr, 0x1ac) as i32,
            duration_input: word(&m.read(0x6c7788, 4)?, 0) as i32,
            moving: vr[0x17a],
            motion_disabled: vr[0x179],
            motion_allowed: vr[0x178],
            camera_mode: m.read(0x6e48c0, 1)?[0],
            application_value_bits: word(&m.read(0x6fe888, 4)?, 0),
            saved_value_bits: word(&vr, 0x1b8),
        };
        let s = Self {
            source,
            owner,
            renderer: word(&vr, 0x54),
            valid: vr[0x148..0x14b].try_into()?,
            motion,
            stage: Stage::Cache,
            frame: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == "c92c35dd68417da337a4ac34b042c6e5c77105f257d412e5ede8093a63c04d1e"
                && self.owner == 0x09e88960
                && self.renderer == 0x09e88e88
                && (1..=3).contains(&self.motion.zoom),
            "camera preparation identity differs"
        );
        ensure!(
            self.frame.is_some() == matches!(self.stage, Stage::Waiting | Stage::Complete),
            "camera frame lifetime differs"
        );
        ensure!(
            self.stage != Stage::Complete
                || self.frame.as_ref().is_some_and(frame::Cursor::complete),
            "camera preparation skipped its motion child"
        );
        Ok(())
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::Cache
    }
    pub fn viewport(&self)->[i32;4]{self.motion.viewport}
    pub(super) fn drawing_allowed(&self)->u8{self.motion.motion_allowed}
    pub(super) fn allow_drawing(&mut self){if self.motion.motion_allowed==0{self.motion.motion_allowed=1;}}

    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn pending(&self) -> Option<Request> {
        match self.stage {
            Stage::Cache if self.valid[(3 - self.motion.zoom) as usize] == 0 => {
                Some(Request::Construct {
                    renderer: self.renderer,
                    zoom: self.motion.zoom,
                })
            }
            Stage::Waiting => self
                .frame
                .as_ref()
                .and_then(frame::Cursor::request)
                .cloned()
                .map(Request::Motion),
            _ => None,
        }
    }
    fn advance(&mut self, view: &mut composition::View) -> Result<bool> {
        self.validate()?;
        view.validate_frame_camera(self.motion.origin, self.motion.zoom, self.motion.level)?;
        match self.stage {
            Stage::Cache => {
                if self.pending().is_some() {
                    return Ok(false);
                }
                self.stage = Stage::Frame;
            }
            Stage::Frame => {
                let frame = frame::Cursor::begin(&mut self.motion);
                self.stage = if frame.complete() {
                    Stage::Complete
                } else {
                    Stage::Waiting
                };
                self.frame = Some(frame);
                view.publish_frame_origin(self.motion.origin);
            }
            Stage::Waiting | Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn compare_native(&self, encoded: &[u8], pc: u32) -> Result<()> {
        use std::io::Read;
        ensure!(
            sims_fixtures::digest(encoded)
                == "1265748772613c4e05d6bd3ea7660f8801e422eec641e20a0f4e6fe6fb7449ad",
            "camera preparation witness differs"
        );
        let mut text = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut text)?;
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let row = rows
            .iter()
            .find(|r| r["kind"] == "world_camera_prepare" && r["pc"] == pc)
            .context("camera preparation native stop absent")?;
        let decode = |v: &serde_json::Value| {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("camera preparation raw bytes absent")?,
            )
        };
        let vr = decode(&row["raw"])?;
        let coordinates = decode(&row["coordinates"])?;
        ensure!(
            row["view"] == self.owner && row["tick"] == 89081,
            "camera preparation native epoch differs"
        );
        ensure!(
            self.valid == vr[0x148..0x14b]
                && self.motion.origin
                    == [word(&coordinates, 0) as i32, word(&coordinates, 4) as i32]
                && self.motion.zoom == word(&coordinates, 8),
            "camera preparation cache/coordinates differ"
        );
        ensure!(
            self.motion.pending == [word(&vr, 0x170) as i32, word(&vr, 0x174) as i32]
                && self.motion.motion_active == word(&vr, 0x1a4)
                && self.motion.motion_policy == word(&vr, 0x1a8)
                && self.motion.motion_duration == word(&vr, 0x1ac) as i32
                && self.motion.saved_value_bits == word(&vr, 0x1b8),
            "camera preparation motion fields differ"
        );
        ensure!(
            [
                self.motion.motion_allowed,
                self.motion.motion_disabled,
                self.motion.moving
            ] == vr[0x178..0x17b],
            "camera preparation flags differ"
        );
        Ok(())
    }
}
impl Owner {
    pub(in crate::original_session::draw_parent) fn bind_camera(
        &mut self,
        source: &[u8],
    ) -> Result<()> {
        ensure!(
            self.at_entry() && self.camera.is_none(),
            "camera preparation already executing/bound"
        );
        let s = State::import(source)?;
        ensure!(s.owner == self.owner, "camera preparation receiver differs");
        self.camera = Some(s);
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn camera_prepare(
        &mut self,
        view: &mut composition::View,
    ) -> Result<bool> {
        ensure!(
            self.phase == Phase::Camera,
            "camera preparation does not own World caller"
        );
        let camera = self
            .camera
            .as_mut()
            .context("camera preparation source absent")?;
        let progress = camera.advance(view)?;
        if camera.complete() {
            self.phase = Phase::PeopleVisibility;
        }
        self.validate()?;
        Ok(progress)
    }
    pub(in crate::original_session::draw_parent) fn camera(&self) -> Option<&State> {
        self.camera.as_ref()
    }
}
