//!53db67 setup. Read model/view identities from B; query the retained current
//! room manager. No observed original return is installed as a service result.
use super::*;
mod mode;
mod segments;
const INPUT: &str = "0d62705421400bc0420315b9b9201710684359f065fba0c27a616c8fd511dcb8";
const MODE_INPUT: &str = "ac29cda65b2320600fc6a2827d49204bab671f29d4c9ac78f08f1af498779d04";
const VECTOR_INPUT: &str = "48bdda26252d0600a22b49fc681cb982a98cff89d1b8279b6aee8731bde22531";
/// B-owned model inputs. Live original observations qualify unchangedness;
/// they are never installed here as callback outputs.
fn model_input(v: &serde_json::Value, m: &Memory, source: &str, model: u32) -> Result<Vec<u8>> {
    let raw = m.read(model, 0x100)?;
    let expected = if source == VECTOR_INPUT {
        let entries = v["first_vector"]["models"].as_array().context("first-vector models absent")?;
        let matching: Vec<_> = entries.iter().filter(|r| r["pointer"] == model).collect();
        ensure!(matching.len() == 1, "geometry model is not uniquely owned by the first vector");
        &matching[0]["raw"]
    } else {
        ensure!(v["setup"]["model"] == model, "geometry child setup model differs");
        &v["setup"]["model_raw"]
    };
    ensure!(decode(expected)? == raw, "geometry child model alias differs");
    Ok(raw)
}
fn segment_input(v: &serde_json::Value, m: &Memory, source: &str, model: u32, raw: &[u8]) -> Result<Vec<u8>> {
    let (begin, end) = (word(raw, 0x30), word(raw, 0x34));
    ensure!(begin <= end && (end-begin)%6 == 0 && end-begin <= 6144, "geometry segment bounds differ");
    let expected = if source == VECTOR_INPUT {
        let entries = v["first_vector"]["models"].as_array().context("first-vector models absent")?;
        let matching: Vec<_> = entries.iter().filter(|r| r["pointer"] == model).collect();
        ensure!(matching.len() == 1 && matching[0]["segment_begin"] == begin && matching[0]["segment_end"] == end,
            "geometry vector segment aliases differ");
        &matching[0]["segments"]
    } else {
        let s = &v["setup"]["segments"];
        ensure!(s["begin"] == begin && s["end"] == end, "geometry segment input aliases differ");
        &s["raw"]
    };
    let bytes = if begin == end { vec![] } else { m.read(begin, (end-begin) as usize)? };
    ensure!(decode(expected)? == bytes, "geometry B segment bytes differ");
    Ok(bytes)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Style,
    StyleReturned,
    ViewReturned,
    RoomCall,
    RoomReturned,
    LightCall,
    Mode,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    model: u32,
    orientation: u32,
    branch: u8,
    process: u32,
    view: u32,
    manager: u32,
    native_rooms: BTreeMap<u16, u32>,
    global_word: u32,
    stage: Stage,
    mask: Option<u32>,
    view_pointer: Option<u32>,
    room_pointer: Option<u32>,
    light: Option<u32>,
    model_light: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<mode::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    segments: Option<segments::State>,
}
impl State {
    pub(super) fn input_request(&self) -> Option<super::super::super::GeometryInput> {
        if let Some(s) = &self.segments { return s.input_request(); }
        (self.stage == Stage::Mode && self.mode.as_ref().and_then(|m|m.point_returned()).is_some())
            .then_some(super::super::super::GeometryInput::Segments)
    }
    pub fn import(encoded: &[u8], request: &level_geometry::Request) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        ensure!(
            matches!(source.as_str(), INPUT | MODE_INPUT | VECTOR_INPUT)
                && request.kind == level_geometry::Kind::First,
            "geometry child setup source/kind differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "geometry child setup epoch differs"
        );
        let m = Memory::new(&v)?;
        let s = &v["setup"];
        let raw = model_input(&v, &m, &source, request.model)?;
        if matches!(source.as_str(), MODE_INPUT | VECTOR_INPUT) {
            segment_input(&v, &m, &source, request.model, &raw)?;
        }
        let process = word(&m.read(0x6e48bc, 4)?, 0);
        let pr = m.read(process, 0x80)?;
        let view = word(&pr, 4);
        let vr = m.read(view, 0x80)?;
        let table = word(&vr, 0);
        let tr = m.read(table, 0x100)?;
        let target = word(&tr, 0x24);
        ensure!(
            s["process"] == process
                && s["process_child_4"] == view
                && view == 0x09e88960u32
                && s["vtable"] == table
                && s["virtual_getter"] == target
                && target == 0x4a6e11,
            "geometry child view aliases differ"
        );
        ensure!(
            sims_fixtures::digest(&m.read(target, 7)?) == "b26137ce17a8386215bb1fde2b53f227d965b04ec16598700dc9cd28de01928f",
            "geometry child internal getter code differs"
        );
        let manager = word(&m.read(word(&pr, 0), 0x80)?, 0x24);
        let mr = m.read(manager, 0x40)?;
        ensure!(
            s["room_manager"] == manager && decode(&s["room_manager_raw"])? == mr,
            "geometry child room manager alias differs"
        );
        let head = word(&mr, 4);
        let hr = m.read(head, 24)?;
        ensure!(
            s["room_head"] == head && decode(&s["room_head_raw"])? == hr,
            "geometry room directory head differs"
        );
        let mut nodes = BTreeMap::new();
        let mut native_rooms = BTreeMap::new();
        for n in s["room_directory"]
            .as_array()
            .context("geometry room directory absent")?
        {
            let p = n["pointer"]
                .as_u64()
                .context("geometry room node pointer absent")? as u32;
            let b = m.read(p, 24)?;
            let id = i16::from_le_bytes(b[16..18].try_into()?) as u16;
            let room = word(&b, 20);
            ensure!(
                decode(&n["raw"])? == b
                    && n["id"].as_i64().map(|v| v as u16) == Some(id)
                    && n["room"] == room,
                "geometry room node alias differs"
            );
            ensure!(
                nodes.insert(p, [word(&b, 8), word(&b, 12)]).is_none()
                    && native_rooms.insert(id, room).is_none(),
                "duplicate geometry room identity"
            );
        }
        let mut pending = vec![word(&hr, 4)];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(p) = pending.pop() {
            if p == 0 || p == head {
                continue;
            }
            ensure!(seen.insert(p), "geometry room directory cycle");
            pending.extend(nodes.get(&p).context("geometry room node omitted")?);
        }
        ensure!(
            seen.len() == nodes.len() && nodes.len() == word(&mr, 8) as usize,
            "geometry room directory incomplete"
        );
        let state = Self {
            segments: None,
            mode: if matches!(source.as_str(), MODE_INPUT | VECTOR_INPUT) {
                Some(mode::State::import(&m, s, &raw, view)?)
            } else {
                None
            },
            source,
            model: request.model,
            orientation: word(&raw, 0x10),
            branch: raw[0x48],
            process,
            view,
            manager,
            native_rooms,
            global_word: word(&m.read(0x6f0bf8, 4)?, 0),
            stage: Stage::Entry,
            mask: None,
            view_pointer: None,
            room_pointer: None,
            light: None,
            model_light: None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn model(&self) -> u32 {
        self.model
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::Entry
    }
    pub fn complete(&self) -> bool {
        self.segments.as_ref().is_some_and(|s| s.complete())
    }
    pub fn bind_visible(&mut self,source:&[u8])->Result<()>{self.validate()?;let hash=sims_fixtures::digest(source);ensure!(matches!(hash.as_str(),MODE_INPUT|VECTOR_INPUT),"visible model source differs");let v:serde_json::Value=serde_json::from_slice(source)?;let m=Memory::new(&v)?;let raw=model_input(&v,&m,&hash,self.model)?;let side=word(&raw,0x18);ensure!(side<2,"visible model side invalid");let light=self.model_light.context("visible model light absent")?;self.segments.as_mut().context("visible segment owner absent")?.bind_visible(light,side as u8)?;self.validate()}
    pub fn bind_materials(&mut self,source:&[u8])->Result<()>{self.validate()?;self.segments.as_mut().context("visible segment absent")?.bind_materials(source)?;self.validate()}
    pub fn try_bind_last_side(&mut self)->Result<bool>{let light=self.light.context("side outside-room light absent")?;let bound=self.segments.as_mut().context("side segment absent")?.try_bind_last_side(light)?;self.validate()?;Ok(bound)}
    pub fn bind_side(&mut self)->Result<()>{let light=self.light.context("side outside-room light absent")?;self.segments.as_mut().context("side segment absent")?.bind_side(light)?;self.validate()}
    pub fn bind_segments(&mut self, source: &[u8], clip: [i32; 4], graph: Option<&sims_sim::lot_presenter::walls::construction::Graph>) -> Result<()> {
        self.validate()?;
        ensure!(
            self.stage == Stage::Mode && self.segments.is_none(),
            "geometry segments binding escaped its mode"
        );
        let point = self
            .mode
            .as_ref()
            .and_then(|m| m.point_returned())
            .context("geometry segment point is not computed")?;
        let zoom = self
            .mode
            .as_ref()
            .and_then(|m| m.zoom())
            .context("geometry mode zoom absent")?;
        self.segments = Some(segments::State::import(
            source, self.model, point, zoom, clip, graph,
        )?);
        self.validate()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.source.as_str(), INPUT | MODE_INPUT | VECTOR_INPUT) && self.view == 0x09e88960,
            "geometry setup identity differs"
        );
        ensure!(
            self.mask.is_some() == !matches!(self.stage, Stage::Entry | Stage::Style),
            "geometry setup mask lifetime differs"
        );
        ensure!(
            self.view_pointer.is_some()
                == !matches!(
                    self.stage,
                    Stage::Entry | Stage::Style | Stage::StyleReturned
                ),
            "geometry setup view lifetime differs"
        );
        ensure!(
            self.room_pointer.is_some()
                == matches!(
                    self.stage,
                    Stage::RoomReturned | Stage::LightCall | Stage::Mode
                ),
            "geometry setup room lifetime differs"
        );
        ensure!(
            self.light.is_some()
                == matches!(
                    self.stage,
                    Stage::RoomCall | Stage::RoomReturned | Stage::LightCall | Stage::Mode
                )
                && self.model_light.is_some() == self.light.is_some(),
            "geometry setup light lifetime differs"
        );
        ensure!(
            self.mode.is_some() == matches!(self.source.as_str(), MODE_INPUT | VECTOR_INPUT),
            "geometry child mode owner differs"
        );
        if let Some(m) = &self.mode {
            m.validate()?;
        }
        if let Some(s) = &self.segments {
            s.validate()?;
            ensure!(
                self.stage == Stage::Mode
                    && self
                        .mode
                        .as_ref()
                        .and_then(|m| m.point_returned())
                        .is_some(),
                "geometry segments lost its point owner"
            );
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        sim: &sims_sim::Simulation,
        camera: &sims_sim::lot_presenter::room_masks::Camera,
        context: &mut sims_vm::scheduler::clock::draw::context::Frame,
        row:Option<&mut crate::original_session::draw_parent::world::paint::row::State>,
        controls:Option<&sims_vm::interface::state::State>,
        graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Entry => self.stage = Stage::Style,
            Stage::Style => {
                self.mask = Some(level_geometry::orientation_mask(self.orientation));
                self.stage = Stage::StyleReturned;
            }
            Stage::StyleReturned => {
                self.view_pointer = Some(self.view.wrapping_add(0xb8));
                self.stage = Stage::ViewReturned;
            }
            Stage::ViewReturned => {
                self.light = Some(1f32.to_bits());
                self.model_light = Some(1f32.to_bits());
                self.stage = Stage::RoomCall;
            }
            Stage::RoomCall => {
                let rooms = &sim
                    .room_environment
                    .as_ref()
                    .context("geometry setup current room manager absent")?
                    .manager
                    .rooms;
                self.room_pointer = Some(if rooms.contains_key(&0) {
                    *self
                        .native_rooms
                        .get(&0)
                        .context("geometry room0 native identity unobserved")?
                } else {
                    0
                });
                self.stage = Stage::RoomReturned;
            }
            Stage::RoomReturned => {
                self.stage = if self.room_pointer == Some(0) {
                    Stage::Mode
                } else {
                    Stage::LightCall
                }
            }
            Stage::LightCall => {
                self.light = Some(
                    sim.room_environment
                        .as_ref()
                        .context("geometry setup current room manager absent")?
                        .manager
                        .rooms
                        .get(&0)
                        .context("geometry setup room0 disappeared")?
                        .effective_light()
                        .to_bits(),
                );
                self.stage = Stage::Mode;
            }
            Stage::Mode => {
                if let Some(s) = &mut self.segments {
                    return s.advance(sim, camera, context,row,controls,self.light.context("geometry outside-room light absent")?,graph);
                }
                let Some(m) = self.mode.as_mut() else {
                    return Ok(false);
                };
                return m.advance(
                    sim,
                    camera.zoom,
                    self.branch,
                    self.orientation,
                    &self.native_rooms,
                    &mut self.model_light,
                );
            }
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_cut(&self, pc: u32) -> bool {
        if let Some(s) = &self.segments {
            if !s.at_entry() {
                return s.at_cut(pc);
            }
        }
        match pc {
            0x53db67 => self.stage == Stage::Entry,
            0x53bfb5 => self.stage == Stage::Style,
            0x53db9e => self.stage == Stage::StyleReturned,
            0x53dba9 => self.stage == Stage::ViewReturned,
            0x466fcc => self.stage == Stage::RoomCall,
            0x53dbd2 => self.stage == Stage::RoomReturned,
            0x4688bf => self.stage == Stage::LightCall,
            0x53dbf2 => {
                self.stage == Stage::Mode && self.mode.as_ref().is_none_or(|m| m.at_entry())
            }
            _ => self.stage == Stage::Mode && self.mode.as_ref().is_some_and(|m| m.at_cut(pc)),
        }
    }
    pub fn compare(&self, q: &serde_json::Value,graphics:GeometryView<'_>) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("geometry setup PC absent")? as u32;
        ensure!(self.at_cut(pc), "geometry setup stage differs at{pc:x}");
        if let Some(s) = &self.segments {
            if !s.at_entry() {
                return s.compare(q,graphics);
            }
        }
        if pc == 0x53db67 {
            return Ok(());
        }
        let p = &q["geometry_prefix"];
        let raw = decode(&p["model_raw"])?;
        ensure!(
            p["model"] == self.model
                && word(&raw, 0x10) == self.orientation
                && raw[0x48] == self.branch
                && p["process"] == self.process
                && p["device"] == self.view
                && word(&decode(&p["process_raw"])?, 4) == self.view
                && word(&decode(&p["device_vtable"])?, 0x24) == 0x4a6e11
                && word(&decode(&p["world_projection"])?, 0) == self.global_word,
            "geometry setup consumed inputs differ"
        );
        let frame = decode(&p["frame"]["raw"])?;
        ensure!(frame.len() == 0x1a0, "geometry setup frame extent differs");
        ensure!(
            word(&frame, 0x178 - 0x7c) == 0
                && word(&frame, 0x178 - 0x70) == self.view
                && word(&frame, 0x178 - 0x90) == self.global_word,
            "geometry setup prologue locals differ"
        );
        let stack = decode(&p["stack"]["raw"])?;
        match pc {
            0x53bfb5 => ensure!(
                word(&stack, 0) == 0x53db9e && word(&stack, 4) == self.orientation,
                "geometry orientation call differs"
            ),
            0x53db9e => ensure!(
                q["registers"]["eax"] == self.mask.unwrap(),
                "geometry orientation result differs"
            ),
            0x53dba9 => ensure!(
                q["registers"]["eax"] == self.view_pointer.unwrap(),
                "geometry internal view getter differs"
            ),
            0x466fcc => ensure!(
                q["registers"]["ecx"] == self.manager
                    && word(&stack, 0) == 0x53dbd2
                    && word(&stack, 4) == 0,
                "geometry room0 lookup differs"
            ),
            0x53dbd2 => ensure!(
                q["registers"]["eax"] == self.room_pointer.unwrap(),
                "geometry room0 identity differs"
            ),
            0x4688bf => ensure!(
                q["registers"]["ecx"] == self.room_pointer.unwrap(),
                "geometry room0 light receiver differs"
            ),
            _ => (),
        }
        if !matches!(self.stage, Stage::Style | Stage::StyleReturned) {
            ensure!(
                word(&frame, 0x178 - 0x48) == self.mask.unwrap(),
                "geometry stored orientation mask differs"
            );
        }
        if self.light.is_some() {
            ensure!(
                word(&frame, 0x178 - 0xd0) == self.view_pointer.unwrap()
                    && word(&frame, 0x178 - 0xa0) == self.model_light.unwrap()
                    && word(&frame, 0x178 - 0xa8) == self.light.unwrap(),
                "geometry stored view/light differs"
            );
        }
        if matches!(self.stage, Stage::LightCall | Stage::Mode) {
            ensure!(
                frame[0x178 - 0x2c..0x178 - 0x28] == [0; 4],
                "geometry setup flags differ"
            );
        }
        if self.stage == Stage::Mode {
            if let Some(m) = &self.mode {
                m.compare(q)?;
            }
        }
        Ok(())
    }
}
