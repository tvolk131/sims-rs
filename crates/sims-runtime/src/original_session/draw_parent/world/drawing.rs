//!4a8bfd..4a8ddc. Context flags, explicit RDTSC record and retained buffer
//! region handoff. The World paint child is a separate, unacknowledged service.
use super::*;
use crate::original_session::draw_parent::framework;
use anyhow::Context as _;
use sims_vm::scheduler::clock::draw::regions;
const SOURCE: &str = "081faa2f70e33729381d92111fc5073de477183a6ddb940d8a4a637a96dd8859";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Marker,
    Append,
    Marked,
    Flags,
    LoopInit,
    Loop,
    Damage,
    DamageDone,
    Test,
    Gather,
    Coalesce,
    Transfer,
    AfterGather,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Timestamp,
    Scroll,
    CameraMode,
    Damage([i32; 4]),
    Coalesce { owner: u32 },
    Region(regions::Request),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InputList {
    header: Vec<u8>,
    used: Vec<u8>,
}
impl InputList {
    fn len(&self) -> usize {
        self.used.len() / 16
    }
    fn rectangle(&self, i: usize) -> Result<[i32; 4]> {
        let b = self
            .used
            .get(16 * i..16 * i + 16)
            .context("drawing input rectangle absent")?;
        Ok(std::array::from_fn(|n| word(b, n * 4) as i32))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    view: u32,
    front_owner: u32,
    back_owner: u32,
    ///Process6f6bc4, used by animator root correction. Absence is unobserved;
    ///584635 publishes a known value without requiring an earlier default.
    #[serde(default,skip_serializing_if="Option::is_none")]
    excluded_person:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    model:Option<paint::model::State>,
    front: [regions::List; 2],
    input: InputList,
    enabled: u32,
    weight: u32,
    camera_mode: u8,
    pending_scroll: [i32; 2],
    damage: [i32; 4],
    record: Option<Record>,
    stage: Stage,
    pass_count: Option<i32>,
    pass: Option<i32>,
    phase: Option<i32>,
    transfer: u8,
    index: usize,
    region: Option<regions::Frame>,
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("drawing bytes absent")?,
    )
}
impl State {
    pub(super) fn bind_model(&mut self,source:&[u8],input:&[u8],classify:&classification::State)->Result<()>{
        ensure!(self.model.is_none(),"model owner already bound");let model=paint::model::State::import(source,input,classify.draw_lists()?)?;
        self.model=Some(model);self.validate()
    }
    pub(super) fn model(&self)->Option<&paint::model::State>{self.model.as_ref()}
    pub(super) fn model_mut(&mut self)->Option<&mut paint::model::State>{self.model.as_mut()}
    pub(super) fn set_excluded_person(&mut self,value:u32){self.excluded_person=Some(value);}
    pub(super) fn excluded_person(&self)->Option<u32>{self.excluded_person}
    pub(super) fn front_present(&self)->bool{self.front_owner!=0}
    pub(super) fn back_present(&self)->bool{self.back_owner!=0}
    pub(super) fn model_regions(&self)->&regions::List{&self.front[1]}
    pub(super) fn model_regions_mut(&mut self)->&mut regions::List{&mut self.front[1]}
    pub(super) fn model_regions_pointer(&self)->u32{self.front_owner+0x1ec}
    pub(super) fn camera_mode(&self) -> u8 { self.camera_mode }
    pub(super) fn region_policy(&self) -> (u32, f32) { (self.enabled, f32::from_bits(self.weight)) }
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "drawing source differs"
        );
        let value: serde_json::Value = serde_json::from_slice(encoded)?;
        let m = Memory::new(&value)?;
        let d = &value["drawing"];
        ensure!(
            value["tick"] == 89073 && value["executes_target_code"] == false,
            "drawing source epoch differs"
        );
        let raw = m.read(0x09e88960, 0x500)?;
        ensure!(raw == decode(&d["raw"])?, "drawing view alias differs");
        let lists = d["lists"]
            .as_array()
            .context("drawing list owners absent")?;
        let find = |label: &str| -> Result<&serde_json::Value> {
            lists
                .iter()
                .find(|x| x["label"] == label)
                .context("drawing region owner absent")
        };
        let list = |label: &str| -> Result<regions::List> {
            let v = find(label)?;
            let h = decode(&v["raw"])?;
            let b = decode(&v["backing"])?;
            ensure!(
                m.read(
                    v["pointer"].as_u64().context("drawing pointer absent")? as u32,
                    24
                )? == h
                    && m.read(word(&h, 0), b.len())? == b,
                "drawing allocation alias differs"
            );
            let l = regions::List {
                header: h,
                backing: b,
            };
            l.validate().map_err(anyhow::Error::msg)?;
            Ok(l)
        };
        let r = find("back_0x498")?;
        let h = decode(&r["raw"])?;
        let used = decode(&r["used_raw"])?;
        ensure!(
            r["backing"].is_null()
                && word(&h, 4) - word(&h, 0) == used.len() as u32
                && used.len() % 16 == 0
                && m.read(word(&h, 0), used.len())? == used,
            "drawing published input differs"
        );
        let state = Self {
            source: SOURCE.into(),
            view: 0x09e88960,
            front_owner: word(&raw, 0x68),
            back_owner: word(&raw, 0x6c),
            excluded_person:None,
            model:None,
            front: [list("front_0x1d4")?, list("front_0x1ec")?],
            input: InputList { header: h, used },
            enabled: word(&m.read(0x6d61b4, 4)?, 0),
            weight: word(&m.read(0x6fe888, 4)?, 0),
            camera_mode: m.read(0x6e48c0, 1)?[0],
            pending_scroll: [word(&raw, 0x170) as i32, word(&raw, 0x174) as i32],
            damage: std::array::from_fn(|n| word(&raw, 0x4cc + n * 4) as i32),
            record: None,
            stage: Stage::Entry,
            pass_count: None,
            pass: None,
            phase: None,
            transfer: 0,
            index: 0,
            region: None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(m)=&self.model{m.validate()?;}
        ensure!(
            self.source == SOURCE
                && self.view == 0x09e88960
                && self.front_owner == 0x09e89030
                && self.back_owner == 0x0313e6a0,
            "drawing retained receivers differ"
        );
        for l in &self.front {
            l.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(
            self.input.header.len() == 24
                && self.input.used.len() % 16 == 0
                && word(&self.input.header, 4) - word(&self.input.header, 0)
                    == self.input.used.len() as u32,
            "drawing input coverage differs"
        );
        ensure!(
            self.record.is_some() == !matches!(self.stage, Stage::Entry | Stage::Marker)
                && self.transfer <= 2,
            "drawing record/transfer lifetime differs"
        );
        ensure!(
            self.region.is_none() || self.stage == Stage::Transfer,
            "drawing rectangle child escaped caller"
        );
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn pending(&self) -> Option<Request> {
        match self.stage {
            Stage::Marker => Some(Request::Timestamp),
            Stage::Loop if self.pending_scroll != [0, 0] => Some(Request::Scroll),
            Stage::Loop if self.camera_mode != 0 => Some(Request::CameraMode),
            Stage::Damage if self.damage[0] < self.damage[2] && self.damage[1] < self.damage[3] => {
                Some(Request::Damage(self.damage))
            }
            Stage::Coalesce => Some(Request::Coalesce {
                owner: self.back_owner + 0x480,
            }),
            Stage::Transfer => self
                .region
                .as_ref()
                .and_then(regions::Frame::pending)
                .cloned()
                .map(Request::Region),
            _ => None,
        }
    }
    pub(super) fn has_record(&self) -> bool {
        self.record.is_some()
    }
    pub(super) fn timestamp(&mut self, time: u64) -> Result<Record> {
        self.validate()?;
        ensure!(self.stage == Stage::Marker, "drawing timestamp not pending");
        let r = Record {
            timestamp: time,
            name: 0x6c72f8,
            argument: 0,
        };
        self.record = Some(r);
        self.stage = Stage::Append;
        self.validate()?;
        Ok(r)
    }
    fn advance(
        &mut self,
        context: &mut context::Frame,
        camera: &mut camera_prepare::State,
        framework: &mut framework::Owner,
        profiler: &mut Buffer,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Entry => {
                camera.allow_drawing();
                self.stage = Stage::Marker;
            }
            Stage::Marker => return Ok(false),
            Stage::Append => {
                profiler
                    .append(self.record.unwrap())
                    .map_err(anyhow::Error::msg)?;
                self.stage = Stage::Marked;
            }
            Stage::Marked => {
                self.pass_count = Some(1);
                self.stage = Stage::Flags;
            }
            Stage::Flags => {
                context.update_flags(0, 15, 1).map_err(anyhow::Error::msg)?;
                self.stage = Stage::LoopInit;
            }
            Stage::LoopInit => {
                self.pass = Some(0);
                self.phase = Some(0);
                self.stage = Stage::Loop;
            }
            Stage::Loop => {
                if self.pending().is_some() {
                    return Ok(false);
                }
                self.stage = Stage::Damage;
            }
            Stage::Damage => {
                if self.pending().is_some() {
                    return Ok(false);
                }
                self.stage = Stage::DamageDone;
            }
            Stage::DamageDone => self.stage = Stage::Test,
            Stage::Test => {
                self.stage = if context.state().flags(0).map_err(anyhow::Error::msg)? & 1 != 0 {
                    Stage::Gather
                } else {
                    Stage::Complete
                };
            }
            Stage::Gather => self.stage = Stage::Coalesce,
            Stage::Coalesce => {
                if !framework
                    .drawing_regions_mut()?
                    .coalesce(self.enabled, f32::from_bits(self.weight))
                    .map_err(anyhow::Error::msg)?
                {
                    return Ok(false);
                }
                self.stage = Stage::Transfer;
            }
            Stage::Transfer => {
                if self.transfer == 2 {
                    self.stage = Stage::AfterGather;
                } else {
                    let target = if self.transfer == 0 { 1 } else { 0 };
                    let count = if self.transfer == 0 {
                        self.input.len()
                    } else {
                        framework.drawing_regions()?.len()
                    };
                    if self.region.is_none() {
                        if self.index == count {
                            self.transfer += 1;
                            self.index = 0;
                            return Ok(true);
                        }
                        let rect = if self.transfer == 0 {
                            self.input.rectangle(self.index)?
                        } else {
                            framework
                                .drawing_regions()?
                                .rectangle(self.index)
                                .map_err(anyhow::Error::msg)?
                        };
                        self.region = Some(regions::Frame::new(rect));
                    }
                    let frame = self.region.as_mut().unwrap();
                    let list = &mut self.front[target];
                    if let Some(r) = frame.pending().cloned() {
                        match r {
                            regions::Request::Append { rectangle } => {
                                if !list.append(rectangle).map_err(anyhow::Error::msg)? {
                                    return Ok(false);
                                }
                            }
                            regions::Request::Coalesce => {
                                if !list
                                    .coalesce(self.enabled, f32::from_bits(self.weight))
                                    .map_err(anyhow::Error::msg)?
                                {
                                    return Ok(false);
                                }
                            }
                        }
                        frame.acknowledge(list, &r).map_err(anyhow::Error::msg)?;
                    }
                    frame
                        .advance(list, self.enabled)
                        .map_err(anyhow::Error::msg)?;
                    if frame.complete() {
                        self.region = None;
                        self.index += 1;
                    }
                }
            }
            Stage::AfterGather => self.stage = Stage::Complete,
            Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4a8bfd => self.stage == Stage::Entry,
            0x4a8c0e => self.stage == Stage::Marker,
            0x4a8c1c => self.stage == Stage::Append,
            0x4a8c29 => self.stage == Stage::Marked,
            0x4a8c39 => self.stage == Stage::Flags,
            0x4a8c3e => self.stage == Stage::LoopInit,
            0x4a8c76 => self.stage == Stage::Loop,
            0x4a8cb2 => self.stage == Stage::Damage,
            0x4a8d05 => self.stage == Stage::DamageDone,
            0x4a8d0d => self.stage == Stage::Test,
            0x4a8d1b => self.stage == Stage::Gather,
            0x5ccfe0 => self.stage == Stage::Coalesce,
            0x5cdeb9 => {
                self.stage == Stage::Transfer
                    && self.transfer == 0
                    && self.index == 0
                    && self.region.is_none()
            }
            0x4a8d20 => self.stage == Stage::AfterGather,
            0x4a8ddc => self.complete(),
            _ => false,
        }
    }
    fn compare_row(
        &self,
        r: &serde_json::Value,
        context: &context::Frame,
        camera: &camera_prepare::State,
        framework: &framework::Owner,
        scene: &scene::State,
        profiler: &Buffer,
        scratch: Option<[u32; 4]>,
    ) -> Result<()> {
        let pc = r["pc"].as_u64().context("drawing native stop absent")? as u32;
        ensure!(
            r["kind"] == "world_drawing_setup" && r["tick"] == 89081 && self.at_native_cut(pc),
            "drawing native stage differs at{pc:x}"
        );
        let raw = decode(&r["raw"])?;
        ensure!(
            raw[0x178] == camera.drawing_allowed()
                && std::array::from_fn::<_, 2, _>(|i| word(&raw, 0x170 + 4 * i) as i32)
                    == self.pending_scroll
                && std::array::from_fn::<_, 4, _>(|i| word(&raw, 0x4cc + 4 * i) as i32)
                    == self.damage,
            "drawing view fields differ"
        );
        let expected: Vec<u32> = decode(&r["context"]["raw"])?
            .chunks_exact(4)
            .map(|b| word(b, 0))
            .collect();
        ensure!(
            context.state().words().as_slice() == expected,
            "drawing shared context differs"
        );
        let lists = r["lists"]
            .as_array()
            .context("drawing native lists absent")?;
        for (label, l) in [
            ("world", &scene.regions),
            ("front_0x1d4", &self.front[0]),
            ("front_0x1ec", &self.front[1]),
            ("back_0x480", framework.drawing_regions()?),
        ] {
            let n = lists
                .iter()
                .find(|v| v["label"] == label)
                .context("drawing native owner absent")?;
            ensure!(
                l.header == decode(&n["raw"])? && l.backing == decode(&n["backing"])?,
                "drawing region header/backing differs: {label}"
            );
        }
        let input = lists
            .iter()
            .find(|v| v["label"] == "back_0x498")
            .context("drawing input absent")?;
        ensure!(
            input["backing"].is_null()
                && self.input.header == decode(&input["raw"])?
                && self.input.used == decode(&input["used_raw"])?,
            "drawing input publication differs"
        );
        ensure!(
            decode(&r["region_enable"])? == self.enabled.to_le_bytes()
                && decode(&r["region_weight"])? == self.weight.to_le_bytes()
                && decode(&r["camera_mode"])? == [self.camera_mode],
            "drawing process fields differ"
        );
        let stack=decode(&r["world_stack"]["raw"])?;
        for(value,offset)in[(self.pass_count,0x30),(self.phase,0x28),(self.pass,0x2c)]{
            if let Some(value)=value{ensure!(word(&stack,0x130-offset)as i32==value,"drawing pass/phase local differs at{pc:x}");}
        }
        if let Some(record) = self.record {
            let stack = decode(&r["world_stack"]["raw"])?;
            let words: Vec<_> = stack[0x130 - 0x48..0x130 - 0x38]
                .chunks_exact(4)
                .map(|b| word(b, 0))
                .collect();
            ensure!(
                words == record.words() && scratch == Some(record.words()),
                "drawing marker scratch alias differs"
            );
            if self.stage != Stage::Append {
                ensure!(
                    profiler.records().last() == Some(&record),
                    "drawing record append differs"
                );
            }
        }
        Ok(())
    }
}
impl Owner {
    pub(in crate::original_session::draw_parent) fn bind_drawing(
        &mut self,
        source: &[u8],
    ) -> Result<()> {
        ensure!(
            self.at_entry() && self.drawing.is_none(),
            "drawing source already executing/bound"
        );
        self.drawing = Some(State::import(source)?);
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn drawing(&self) -> Option<&State> {
        self.drawing.as_ref()
    }
    pub(in crate::original_session::draw_parent) fn advance_drawing(
        &mut self,
        framework: &mut framework::Owner,
        profiler: &mut Buffer,
    ) -> Result<bool> {
        ensure!(
            self.phase == Phase::Drawing,
            "drawing setup does not own World"
        );
        let state = self.drawing.as_mut().context("drawing source absent")?;
        let progress = state.advance(
            self.context.as_mut().context("drawing context absent")?,
            self.camera.as_mut().context("drawing camera absent")?,
            framework,
            profiler,
        )?;
        if state.complete() {
            self.phase = Phase::Painting;
        }
        self.validate()?;
        Ok(progress)
    }
    pub(in crate::original_session::draw_parent) fn compare_drawing(
        &self,
        r: &serde_json::Value,
        framework: &framework::Owner,
        scene: &scene::State,
        profiler: &Buffer,
    ) -> Result<()> {
        self.drawing
            .as_ref()
            .context("drawing source absent")?
            .compare_row(
                r,
                self.context.as_ref().context("drawing context absent")?,
                self.camera.as_ref().context("drawing camera absent")?,
                framework,
                scene,
                profiler,
                self.scratch,
            )
    }
}
