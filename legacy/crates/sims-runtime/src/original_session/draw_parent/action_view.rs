//! Frozen B action-window owners plus the complete4d28df caller. The mode
//! getter borrows the actual CPState; it never creates a second mutable mode.
use super::{Memory, Result, ensure, word};
use serde::{Deserialize, Serialize};
use sims_vm::{interface::state, scheduler::clock::draw::actions};
use std::collections::BTreeMap;
mod records;
mod collection;
mod publication;
mod tail;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owner {
    frame: actions::Frame,
    state: actions::State,
    retired: Vec<u32>,
    /// The source getter target for each admitted parent owner.
    parent_targets: BTreeMap<u32, u32>,
    /// Native vector allocation metadata stays owned, not inferred from Vec's
    /// host allocation. The unimplemented record/allocator children may not
    /// mutate these headers by merely receiving a Done acknowledgement.
    vector_headers: Vec<[u32; 3]>,
    #[serde(default)]
    resources:Option<records::Resources>,
    #[serde(default)]
    collection:Option<collection::Owner>,
    #[serde(default)]
    publication:Option<publication::Owner>,
    #[serde(default)]
    tail:Option<tail::Owner>,
}
pub(super) fn import(source: &[u8]) -> Result<Owner> {
    ensure!(
        sims_fixtures::digest(source)
            == "7c4833dc4de5a51f5b10c24880a67b71cbe6a4f604a44aff75162ea540727ed6",
        "action-window source differs"
    );
    let v: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        v["schema"] == 1
            && v["tick"] == 89073
            && v["executes_target_code"] == false
            && v["native_parity_credit"] == 0,
        "action-window source epoch differs"
    );
    let m = Memory::new(&v)?;
    for (p, n, h) in [
        (
            0x4d28df,
            0x3c4,
            "56cbc215fec21d3119a226aa15b46ad4035d095c9b71c01e412b3aa8897f032b",
        ),
        (
            0x416920,
            4,
            "916ae10aa3fd76a4d5b25b2fb66e9df0fd04957c7ab10930268d1ed7808be689",
        ),
        (
            0x436e78,
            4,
            "68b70ba1e6ad2356c69cac8e846649e599945994fa7690a0bf4af28fa25c24a5",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(p, n)?) == h,
            "action-window caller/getter code differs"
        );
    }
    let app = m.read(0x0315a250, 0x11c0)?;
    ensure!(
        word(&app, 0x11c) == 0x03115a10,
        "action-window CPState alias differs"
    );
    let _source_mode = word(&m.read(0x03115a10, 0x220)?, 8);
    let mut vector_headers = Vec::new();
    let mut vector = |address, stride| -> Result<Vec<u8>> {
        let h = m.read(address, 12)?;
        let [begin, end, cap] = std::array::from_fn(|i| word(&h, i * 4));
        ensure!(
            begin <= end && end <= cap && (end - begin) % stride == 0 && cap - begin <= 65536,
            "action-window native vector extent differs"
        );
        vector_headers.push([begin, end, cap]);
        m.read(begin, (end - begin) as usize)
    };
    let windows = vector(0x0315a250 + 0x117c, 4)?;
    let retired = vector(0x0315a250 + 0x1188, 4)?;
    let records = vector(0x6f0f18, 28)?
        .chunks_exact(28)
        .map(|b| b.try_into().unwrap())
        .collect();
    let scratch = vector(0x6f0f08, 4)?;
    let pointers = |raw: Vec<u8>| raw.chunks_exact(4).map(|b| word(b, 0)).collect::<Vec<_>>();
    let windows = pointers(windows);
    let retired = pointers(retired);
    let scratch = pointers(scratch);
    let mut headers = BTreeMap::new();
    let mut parent_targets = BTreeMap::new();
    for &window in windows.iter().chain(&retired) {
        let h = m.read(window, 0x124)?;
        let table = word(&h, 0);
        let target = word(&m.read(table + 0x18, 4)?, 0);
        ensure!(target == 0x436e78, "action-window parent getter differs");
        parent_targets.insert(window, target);
        headers.insert(window, h);
    }
    let owner = Owner {
        frame: actions::Frame::at_entry(),
        state: actions::State {
            initialized: m.read(0x6f0f24, 1)?[0],
            windows,
            records,
            scratch,
            headers,
            hint_pending: app[0x1194],
            hint_token: word(&app, 0x1198),
            hint_point: [word(&app, 0x11a0), word(&app, 0x11a4)],
        },
        parent_targets,
        vector_headers,
        retired,
        resources:None,
        collection:None,
        publication:None,
        tail:None,
    };
    owner.validate()?;
    Ok(owner)
}
impl Owner {
    pub(super) fn validate(&self) -> Result<()> {
        self.frame
            .validate_state(&self.state)
            .map_err(anyhow::Error::msg)?;
        if let Some(resources)=&self.resources {
            resources.validate(&self.state.records)?;
            resources.validate_temporary(self.collection.as_ref().and_then(collection::Owner::temporary),&self.state.records,word(&self.state.headers[&0x0b7c7cb0],0xdc),&self.publication.as_ref().map_or_else(Vec::new,publication::Owner::adjustments))?;
            if self.publication.is_none(){ensure!(resources.images.get(&0x0bb6d5e0).copied().unwrap_or(0)>=1,"unbound image lifetime advanced");}
        }
        if let Some(collection)=&self.collection {collection.validate()?;}
        if let Some(publication)=&self.publication {publication.validate(&self.state.headers)?;publication.validate_counts(&self.state.headers,&self.resources.as_ref().ok_or_else(||anyhow::anyhow!("publication resources missing"))?.images)?;}
        if let Some(tail)=&self.tail {tail.validate(&self.vector_headers,&self.state)?;}
        let record_end=if self.resources.as_ref().is_some_and(records::Resources::cleared){0x0ebdd378+self.state.records.len()as u32*28}else{0x0ebdd394};
        ensure!(
            self.vector_headers
                == [
                    [0x0edc8d90, if self.tail.is_some(){0x0edc8d90+self.state.windows.len()as u32*4}else{0x0edc8d94}, 0x0edc8d98],
                    [0x0ea8a5a0, 0x0ea8a5a4, 0x0ea8a5a4],
                    [0x0ebdd378, record_end, 0x0ebdd3b0],
                    [0x0ea8a5a8, if self.tail.is_some(){0x0ea8a5a8+self.state.scratch.len()as u32*4}else{0x0ea8a5ac}, 0x0ea8a5b0],
                ]
                && self.retired == [0x0b4c64c0]
                && self.parent_targets
                    == BTreeMap::from([(0x0b7c7cb0, 0x436e78), (0x0b4c64c0, 0x436e78)])
                && self.state.headers.values().all(|h| word(h, 0) == 0x6689c0),
            "action-window retained source metadata differs"
        );
        Ok(())
    }
    pub(super) fn at_entry(&self) -> bool {
        self.frame.is_at_entry()
    }
    pub(super) fn bind_records(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry()&&self.resources.is_none(),"record resource binding already started");
        self.resources=Some(records::import(source,&self.state.records)?);self.validate()
    }
    pub(super) fn record_pending(&self)->Option<actions::clear::Request> {self.resources.as_ref().and_then(records::Resources::pending)}
    pub(super) fn bind_collection(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry()&&self.collection.is_none(),"collection binding already started");
        self.collection=Some(collection::import(source)?);self.validate()
    }
    pub(super) fn bind_record_build(&mut self,source:&[u8])->Result<()> {
        ensure!(self.frame == actions::Frame::at_entry(),"record build binding already executing");
        self.resources.as_mut().ok_or_else(||anyhow::anyhow!("record resource owner absent"))?.bind_constructor(source)
    }
    pub(super) fn bind_services(&mut self,code:&[u8],object:&[u8],pixels:&[u8])->Result<()> {
        ensure!(self.at_entry(),"record service binding already executing");
        self.resources.as_mut().ok_or_else(||anyhow::anyhow!("record resource owner absent"))?.bind_services(code,object,pixels)
    }
    pub(super) fn bind_publication(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry()&&self.publication.is_none()&&self.resources.is_some(),"publication binding already started/missing resource owner");
        self.publication=Some(publication::import(source,&self.state.headers)?);self.validate()
    }
    pub(super) fn bind_tail(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry()&&self.tail.is_none(),"action tail binding already started");
        self.tail=Some(tail::import(source,&self.vector_headers)?);self.validate()
    }
    pub(super) fn image_pending(&self)->Option<&actions::image::Request>{self.publication.as_ref().and_then(publication::Owner::pending)}
    pub(super) fn advance_for_program(&mut self,controls:&state::State,simulation:&mut sims_sim::Simulation,program:&sims_vm::Program)->Result<Option<actions::Request>> {
        for _ in 0..8194 {
            let request=self.advance_for_simulation(controls,simulation)?;
            if let Some(actions::Request::Token{record})=request {
                let actor=self.collection.as_ref().ok_or_else(||anyhow::anyhow!("record token person bindings absent"))?.actor_for_node(actions::record::word(&record,12))?;
                let action=simulation.vm.world.actions.as_ref().and_then(|s|s.actors.get(&actor)).ok_or_else(||anyhow::anyhow!("record token action owner absent"))?;
                let value=action.active.as_ref().map_or(0,|a|a.id);
                self.frame.acknowledge(&mut self.state,actions::Request::Token{record},actions::Reply::Value(value)).map_err(anyhow::Error::msg)?;
                continue;
            }
            if matches!(request,Some(actions::Request::Collect{..})) {
                if let (Some(collection),Some(resources))=(&mut self.collection,&mut self.resources) {
                    if collection.service_program(resources,simulation,program,&mut self.state.records)? {
                        self.vector_headers[2][1]=self.vector_headers[2][0]+self.state.records.len()as u32*28;
                        continue;
                    }
                }
            }
            if let Some(actions::Request::Image{window,image})=request {
                if let (Some(publication),Some(resources))=(&mut self.publication,&mut self.resources) {
                    if publication.advance(window,image,&mut self.state.headers,resources)? {
                        self.frame.acknowledge(&mut self.state,actions::Request::Image{window,image},actions::Reply::Done).map_err(anyhow::Error::msg)?;
                    }else{return Ok(request);}
                    continue;
                }
            }
            if let Some(actions::Request::CaptionDirty{record})=request {
                if self.tail.is_some(){
                    let actor=self.collection.as_ref().ok_or_else(||anyhow::anyhow!("dirty getter person bindings absent"))?.actor_for_node(actions::record::word(&record,12))?;
                    let active=simulation.vm.world.actions.as_ref().and_then(|s|s.actors.get(&actor)).and_then(|a|a.active.as_ref()).ok_or_else(||anyhow::anyhow!("dirty getter active action absent"))?;
                    let value=actions::tail::caption_dirty(active.flags);
                    self.frame.acknowledge(&mut self.state,actions::Request::CaptionDirty{record},actions::Reply::Value(value)).map_err(anyhow::Error::msg)?;
                    continue;
                }
            }
            return Ok(request);
        }
        anyhow::bail!("record services work bound exceeded")
    }
    pub(super) fn collection_pending(&self)->Option<&actions::collect::Request> {self.collection.as_ref().and_then(collection::Owner::pending)}
    pub(super) fn advance_for_simulation(&mut self,controls:&state::State,simulation:&sims_sim::Simulation)->Result<Option<actions::Request>> {
        for _ in 0..8194 {
            let request=self.advance(controls)?;
            match request {
                Some(actions::Request::SelectedPerson)=>{
                    let index=crate::original_session::person_publication::selected_index(simulation)?;
                    self.frame.acknowledge(&mut self.state,actions::Request::SelectedPerson,actions::Reply::Value(index as u32)).map_err(anyhow::Error::msg)?;
                }
                Some(actions::Request::Collect{..})=>{
                    let Some(collection)=&mut self.collection else{return Ok(request);};
                    if !collection.service_scalar(simulation,&mut self.state.records)? {return Ok(request);}
                }
                _=>return Ok(request),
            }
        }
        anyhow::bail!("collection scalar work bound exceeded")
    }
    pub(super) fn complete(&self) -> bool {
        self.frame.complete()
    }
    pub(super) fn pending(&self) -> Option<&actions::Request> {
        self.frame.pending()
    }
    pub(super) fn state(&self) -> &actions::State {
        &self.state
    }
    pub(super) fn advance(&mut self, controls: &state::State) -> Result<Option<actions::Request>> {
        self.validate()?;
        controls.validate().map_err(anyhow::Error::msg)?;
        for _ in 0..8194 {
            let request=self.frame.advance(&mut self.state).map_err(anyhow::Error::msg)?;
            if let Some(tail)=&self.tail {tail.sync_caller_erase(&mut self.vector_headers,&self.state)?;}
            let Some(request)=request else{return Ok(None);};
            if let Some(tail)=&mut self.tail {
                if tail.service(&request,&mut self.vector_headers,&mut self.state)? {
                    self.frame.acknowledge(&mut self.state,request,actions::Reply::Done).map_err(anyhow::Error::msg)?;
                    continue;
                }
            }
            let reply = match request {
                actions::Request::Mode => actions::Reply::Value(controls.mode as u32),
                actions::Request::Parent { window } => {
                    ensure!(
                        self.parent_targets.get(&window) == Some(&0x436e78),
                        "action-window parent callee needs a qualified owner"
                    );
                    actions::Reply::Value(word(&self.state.headers[&window], 0x40))
                }
                actions::Request::ClearRecords if self.resources.is_some()=>{
                    if !self.resources.as_mut().unwrap().advance(&mut self.state.records)? {return Ok(Some(request));}
                    self.vector_headers[2][1]=self.vector_headers[2][0];actions::Reply::Done
                }
                actions::Request::Collect{selected} if self.collection.is_some()=>{
                    let collection=self.collection.as_mut().unwrap();collection.advance(selected)?;
                    if let Some(resources)=&mut self.resources {
                        if collection.service_storage(resources,&mut self.state.records)? {continue;}
                    }
                    if !collection.complete() {return Ok(Some(request));}
                    actions::Reply::Records(std::mem::take(&mut self.state.records))
                }
                _ => return Ok(Some(request)),
            };
            self.frame
                .acknowledge(&mut self.state, request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        anyhow::bail!("action-window getter work bound exceeded")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::original_session::{completed_animations::bytes, completed_controls, draw_parent};
    use anyhow::Context;
    use sims_vm::scheduler::clock::{draw, profiler::Buffer};
    use std::io::Read;
    fn decode(path: &std::path::Path) -> Result<Vec<serde_json::Value>> {
        let mut raw = String::new();
        flate2::read::GzDecoder::new(std::fs::File::open(path)?).read_to_string(&mut raw)?;
        raw.lines()
            .map(|l| serde_json::from_str(l).map_err(Into::into))
            .collect()
    }
    #[test]
    #[ignore = "private frozen action-window owners and original boundary"]
    fn source_bound_child_retains_caller_and_borrows_actual_controls() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source =
            std::fs::read(root.join(
                "fixtures/private/goth-action-refresh-20261031-first/action-source-first.json",
            ))?;
        let draw_source =
            std::fs::read(root.join(
                "fixtures/private/goth-render-parent-20261031-first/render-source-second.json",
            ))?;
        let children = std::fs::read(root.join(
            "fixtures/private/goth-draw-services-20261031-first/services-source-first.json",
        ))?;
        let trace = decode(&root.join(
            "fixtures/private/goth-action-image-witness-20261031-first/action-image-trace.jsonl.gz",
        ))?;
        let at = |pc| {
            trace
                .iter()
                .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == pc)
                .context("original action-view point absent")
        };
        let before = at(0x4d28dfu32)?;
        let native=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-00");
        let mut raw = String::new();
        flate2::read::GzDecoder::new(std::fs::File::open(native.join("observation.json.gz"))?)
            .read_to_string(&mut raw)?;
        let observation: serde_json::Value = serde_json::from_str(&raw)?;
        let memory = std::fs::read(native.join("acknowledged.mem"))?;
        let memory = sims_fixtures::readset::Readset::parse(&memory)?;
        let controls = completed_controls::captured::import(&observation, &memory)?.controls;
        ensure!(controls.mode == 2, "captured mode differs");
        let mut state = draw_parent::import(&draw_source)?;
        state.bind_child_source(&children)?;
        state.bind_action_source(&source)?;
        let owner = state.actions.as_ref().unwrap();
        for w in before["action_images"].as_array().unwrap() {
            let id = w["address"].as_u64().unwrap() as u32;
            assert_eq!(owner.state.headers[&id], bytes(w["raw"].as_str().unwrap())?);
        }
        let record = before["action_vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["label"] == "records")
            .unwrap();
        assert_eq!(
            owner.state.records.concat(),
            bytes(record["raw"].as_str().unwrap())?
        );
        let mut profiler = Buffer::from_captured(vec![], 32).map_err(anyhow::Error::msg)?;
        loop {
            let request = state.advance()?.context("Draw returned too early")?;
            if request == draw::Request::ApplicationView {
                break;
            }
            if state.service_child(&mut profiler)? {
                continue;
            }
            let reply = match request {
                draw::Request::Timestamp { .. } => draw::Reply::Timestamp(0),
                draw::Request::FpsClock => draw::Reply::Clock(758330),
                _ => anyhow::bail!("unexpected Draw child"),
            };
            state.acknowledge(request, reply, &mut profiler)?;
        }
        let before_headers = state.action_view_state().unwrap().headers.clone();
        assert_eq!(
            state.advance_action_view(&controls)?,
            Some(actions::Request::ClearRecords)
        );
        assert_eq!(state.action_view_state().unwrap().headers, before_headers);
        assert!(
            state
                .acknowledge(
                    draw::Request::ApplicationView,
                    draw::Reply::Done,
                    &mut profiler
                )
                .is_err()
        );
        let encoded = postcard::to_allocvec(&state)?;
        let mut restored: draw_parent::State = postcard::from_bytes(&encoded)?;
        assert_eq!(state, restored);
        let mut changed = controls.clone();
        changed.mode = 1;
        // The original getter was already consumed; a cold child cannot reread
        // mode or switch branches while record destruction remains pending.
        assert_eq!(
            restored.advance_action_view(&changed)?,
            Some(actions::Request::ClearRecords)
        );
        assert_eq!(postcard::to_allocvec(&restored)?, encoded);
        // A genuinely fresh caller reads the current borrowed mode. Its first
        // attached old window must hide before hierarchy removal.
        let mut owner = import(&source)?;
        assert_eq!(
            owner.advance(&changed)?,
            Some(actions::Request::Show {
                window: 0x0b7c7cb0,
                visible: false
            })
        );
        Ok(())
    }
}
