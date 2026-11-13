//! B-owned iterator words. Child model records are deliberately not imported:
//! three second-vector records already differ at the original C8 entry.
use super::*;
mod child;
mod corner;
use sims_sim::lot_presenter::level_geometry::{self, Frame, Words};
use std::collections::BTreeMap;
const INPUT: &str = "e72a81a206221a6d4f64474ce15f464ae343a8c893c1b817ca2532b0465953b0";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    model: u32,
    words: OwnedWords,
    frame: Frame,
    #[serde(default,skip_serializing_if="Option::is_none")]
    child:Option<child::State>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    corner:Option<corner::State>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OwnedWords(BTreeMap<u32, u32>);
impl Words for OwnedWords {
    fn word(&self, address: u32) -> std::result::Result<u32, String> {
        self.0
            .get(&address)
            .copied()
            .ok_or_else(|| format!("geometry word {address:x} is unobserved"))
    }
}
impl State {
    pub(super) fn input_request(&self) -> Option<super::super::GeometryInput> {
        use super::super::GeometryInput;
        if let Some(c) = &self.child { return c.input_request(); }
        if self.corner.is_some() { return None; }
        self.frame.pending().map(|r| match r.kind {
            level_geometry::Kind::First => GeometryInput::First,
            level_geometry::Kind::Second => GeometryInput::Corner,
        })
    }
    pub fn import(encoded: &[u8], model: u32, level: i32) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == INPUT,
            "geometry iterator source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false && level == 1,
            "geometry iterator source epoch/level differs"
        );
        let m = Memory::new(&v)?;
        let g = &v["geometry"];
        ensure!(
            g["model"] == model && g["level"] == level,
            "geometry iterator model/level alias differs"
        );
        let body = m.read(model, 0x40)?;
        ensure!(decode(&g["raw"])? == body, "geometry model alias differs");
        let groups = g["groups"]
            .as_array()
            .context("geometry vector sources absent")?;
        ensure!(groups.len() == 2, "geometry vector count differs");
        let mut words = BTreeMap::new();
        for (group, offset) in groups.iter().zip([0x10, 8]) {
            let header = word(&body, offset);
            let h = m.read(header, 16)?;
            ensure!(
                group["model_offset"] == offset
                    && group["header_pointer"] == header
                    && decode(&group["header"])? == h,
                "geometry header alias differs"
            );
            let (begin, end) = (word(&h, 0), word(&h, 4));
            ensure!(
                begin <= end && (end - begin) % 4 == 0 && end - begin <= 2048,
                "geometry vector bounds unsupported"
            );
            words.insert(model + offset as u32, header);
            words.insert(header, begin);
            words.insert(header + 4, end);
            let entries = if begin == end {
                vec![]
            } else {
                m.read(begin, (end - begin) as usize)?
            };
            ensure!(
                decode(&group["elements"])? == entries,
                "geometry elements alias differs"
            );
            for (i, e) in entries.chunks_exact(4).enumerate() {
                words.insert(begin + 4 * i as u32, word(e, 0));
            }
        }
        let s = Self {
            source: INPUT.into(),
            model,
            words: OwnedWords(words),
            frame: Frame::new(model, level),
            child:None,
            corner:None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.source == INPUT, "geometry iterator identity differs");
        if let Some(c)=&self.child {c.validate()?;ensure!(if c.complete(){self.frame.at_child_return(level_geometry::Kind::First)}else{self.frame.pending().is_some_and(|r|r.kind==level_geometry::Kind::First && r.model==c.model())},"geometry child lost actual iterator call/return");}
        if let Some(c)=&self.corner {c.validate()?;ensure!(self.child.is_none()&&if c.complete(){self.frame.at_child_return(level_geometry::Kind::Second)}else{self.frame.pending().is_some_and(|r|r.kind==level_geometry::Kind::Second&&r.model==c.model())},"corner lost its iterator call/return");}
        self.frame.validate().map_err(anyhow::Error::msg)
    }
    pub fn at_entry(&self) -> bool {
        self.frame.at_entry()
    }
    pub fn complete(&self) -> bool {
        self.frame.complete()
    }
    pub fn pending(&self) -> Option<&level_geometry::Request> {
        self.frame.pending()
    }
    pub fn advance(&mut self) -> Result<bool> {
        let progress=self.frame.advance(&self.words).map_err(anyhow::Error::msg)?;
        if progress&&self.child.as_ref().is_some_and(|c|c.complete()) {self.child=None;}
        if progress&&self.corner.as_ref().is_some_and(|c|c.complete()) {self.corner=None;}
        Ok(progress)
    }
    pub fn bind_child(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.child.is_none()&&self.corner.is_none(),"geometry child already bound");
        self.child=Some(child::State::import(source,self.frame.pending().context("geometry child call absent")?)?);self.validate()
    }
    pub fn bind_corner(&mut self,source:&[u8],setup:&[u8],graph:&sims_sim::lot_presenter::walls::construction::Graph)->Result<()>{self.validate()?;ensure!(self.child.is_none(),"non-corner geometry already bound");let fresh=corner::State::import(source,setup,self.frame.pending().context("corner call absent")?,graph)?;if let Some(old)=&mut self.corner{old.enrich(fresh)?;}else{self.corner=Some(fresh);}self.validate()}
    pub fn bind_segments(&mut self,source:&[u8],clip:[i32;4],graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>)->Result<()>{self.validate()?;self.child.as_mut().context("geometry child absent")?.bind_segments(source,clip,graph)?;self.validate()}
    pub fn bind_visible(&mut self,source:&[u8])->Result<()>{self.validate()?;self.child.as_mut().context("geometry child absent")?.bind_visible(source)?;self.validate()}
    pub fn bind_materials(&mut self,source:&[u8])->Result<()>{self.validate()?;self.child.as_mut().context("geometry child absent")?.bind_materials(source)?;self.validate()}
    pub fn try_bind_last_side(&mut self)->Result<bool>{let bound=self.child.as_mut().context("geometry child absent")?.try_bind_last_side()?;self.validate()?;Ok(bound)}
    pub fn bind_side(&mut self)->Result<()>{self.child.as_mut().context("geometry child absent")?.bind_side()?;self.validate()}
    pub fn advance_child(&mut self,sim:&sims_sim::Simulation,camera:&sims_sim::lot_presenter::room_masks::Camera,context:&mut sims_vm::scheduler::clock::draw::context::Frame,row:Option<&mut crate::original_session::draw_parent::world::paint::row::State>,controls:Option<&sims_vm::interface::state::State>,graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>,clip:[i32;4])->Result<bool>{self.validate()?;if let Some(c)=&mut self.corner{let progress=c.advance(sim,camera,context,row,clip)?;if c.complete(){if let Some(r)=self.frame.pending().cloned(){self.frame.acknowledge(&r).map_err(anyhow::Error::msg)?;}}self.validate()?;return Ok(progress)}let p=self.child.as_mut().context("geometry child setup absent")?.advance(sim,camera,context,row,controls,graph)?;if self.child.as_ref().unwrap().complete(){if let Some(request)=self.frame.pending().cloned(){self.frame.acknowledge(&request).map_err(anyhow::Error::msg)?;}}self.validate()?;Ok(p)}
    pub fn child_at_cut(&self,pc:u32)->bool {self.child.as_ref().is_some_and(|c|c.at_cut(pc))||self.corner.as_ref().is_some_and(|c|c.at_cut(pc))}
    pub fn child_at_entry(&self)->bool {self.child.as_ref().is_none_or(|c|c.at_entry())&&self.corner.as_ref().is_none_or(|c|c.at_entry())}
    pub fn compare_child(&self,q:&serde_json::Value,graphics:GeometryView<'_>)->Result<()> {if let Some(c)=&self.corner{c.compare(q,graphics)?;if c.complete(){ensure!(q["registers"]["esi"]==self.frame.cursor()&&q["registers"]["edi"]==self.frame.slot()&&q["registers"]["ebx"]==1,"corner caller register restoration differs");}return Ok(())}let c=self.child.as_ref().context("geometry child setup absent")?;c.compare(q,graphics)?;if c.complete(){ensure!(q["registers"]["esi"]==self.frame.cursor()&&q["registers"]["edi"]==self.model&&q["registers"]["ebx"]==1,"geometry child caller register restoration differs");}Ok(())}
    pub fn compare_inputs(&self, q: &serde_json::Value) -> Result<()> {
        self.validate()?;
        let g = &q["level_geometry"];
        ensure!(
            g["model"] == self.model,
            "native geometry iterator model differs"
        );
        let body = decode(&g["raw"])?;
        let groups = g["groups"]
            .as_array()
            .context("native geometry vectors absent")?;
        ensure!(groups.len() == 2, "native geometry vector count differs");
        for (group, offset) in groups.iter().zip([0x10, 8]) {
            let header = self
                .words
                .word(self.model + offset)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                word(&body, offset as usize) == header && group["header_pointer"] == header,
                "native geometry indirect slot differs"
            );
            let h = decode(&group["header"])?;
            let (begin, end) = (
                self.words.word(header).map_err(anyhow::Error::msg)?,
                self.words.word(header + 4).map_err(anyhow::Error::msg)?,
            );
            ensure!(
                word(&h, 0) == begin && word(&h, 4) == end,
                "native geometry iterator bounds differ"
            );
            let native = decode(&group["elements"])?;
            let computed = (begin..end)
                .step_by(4)
                .map(|a| self.words.word(a))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::msg)?;
            ensure!(
                computed.iter().flat_map(|v| v.to_le_bytes()).eq(native),
                "native geometry elements differ"
            );
        }
        Ok(())
    }
    pub fn compare_call(&self, q: &serde_json::Value, rectangle: [i32; 4]) -> Result<()> {
        self.validate()?;
        let r = self.pending().context("geometry child not pending")?;
        let child = &q["level_geometry_child"];
        let a = child["arguments"]
            .as_array()
            .context("geometry child arguments absent")?;
        let raw = decode(&child["rectangle"])?;
        let parent = decode(&q["post_base"]["parent_frame"]["raw"])?;
        ensure!(
            raw.len() == 16
                && std::array::from_fn::<_, 4, _>(|i| word(&raw, 4 * i) as i32) == rectangle,
            "geometry child rectangle differs"
        );
        ensure!(
            q["pc"] == r.kind.entry()
                && q["registers"]["ecx"] == r.model
                && child["model"] == r.model
                && a.len() == 4
                && a[0] == r.kind.return_pc()
                && a[1] == word(&parent, 0x58)
                && a[2] == q["context"]["pointer"]
                && a[3] == 0
                && child["context"] == q["context"]["raw"],
            "geometry child ABI differs"
        );
        ensure!(
            q["registers"]["esi"] == self.frame.cursor()
                && q["registers"]["eax"] == self.frame.header()
                && q["registers"]["edi"]
                    == if r.kind == level_geometry::Kind::First {
                        self.model
                    } else {
                        self.frame.slot()
                    },
            "geometry child iterator registers differ"
        );
        Ok(())
    }
}
