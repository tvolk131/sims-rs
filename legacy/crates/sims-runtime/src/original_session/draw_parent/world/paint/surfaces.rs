//! Retained first target acquisition. Only DirectDraw descriptor writes and
//! HRESULT are external inputs; backend/plane counts and target publication
//! are computed by the shared Rust cursors.
use super::*;
use sims_vm::scheduler::clock::draw::targets::{self, backend};
use std::collections::BTreeMap;
mod end;
mod copy;
const SOURCE: &str = "d30b2cc2c1e45955414436e3d5cafe3e666b6b0bd994d6cdab9e73b9a091976b";
const TRACE: &str = "2404b08975f88c3aed37fb32bf2da4ec901526774aacb4d36cd91017485fac52";
const STREAM:&str="007d85a637c903b8a792005dece5a08e92610eba4266ba8dcc93586ddaf39bfb";
const END_STREAM:&str="e78373a994f076f7960df094839c737ba44b5997f33ad25b709d7cd5791bdeea";
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
struct Front {input:String,surface:u32,started:bool}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Reply {
    backend: u32,
    interface: u32,
    offset: usize,
    before: Vec<u8>,
    after: Vec<u8>,
    hresult: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Plane {
    raw: Vec<u8>,
    backend: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum LockStage {
    Entry,
    Backend,
    Returned,
    Count,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Work {
    Lock {
        plane: u32,
        flags: u32,
        stage: LockStage,
        frame: Option<backend::Lock>,
    },
    Getter {
        plane: u32,
        pitch: bool,
    },
    Unlock {
        plane: u32,
        flags: u32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    trace: String,
    pub fields: targets::Fields,
    middle: Vec<u8>,
    planes: BTreeMap<u32, Plane>,
    backends: BTreeMap<u32, backend::Fields>,
    replies: Vec<Reply>,
    input: usize,
    begin: Option<targets::Begin>,
    work: Option<Work>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    end:Option<end::End>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    copy:Option<copy::Copy>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    front:Option<Front>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    post_end:Option<end::End>,
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("surface bytes absent")?,
    )
}
fn set(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
impl State {
    pub(super) fn bind_front(&mut self,source:&[u8],input:&[u8])->Result<()>{
        self.validate()?;let input_hash=sims_fixtures::digest(input);ensure!(sims_fixtures::digest(source)==SOURCE&&(input_hash==STREAM||input_hash==END_STREAM||input_hash==super::model::INPUT),"front platform source differs");
        if let Some(f)=&self.front{ensure!(f.input==input_hash,"front input rebinding differs");return Ok(());}
        let v:serde_json::Value=serde_json::from_slice(source)?;let memory=Memory::new(&v)?;let window=memory.read(0x09e88960,0x520)?;
        let inputs:serde_json::Value=serde_json::from_slice(input)?;let records=inputs["replies"].as_array().context("front platform replies absent")?;
        ensure!(records.len()==2,"front platform input extent differs");let mut replies=vec![];
        for r in records{
            let number=|key:&str|->Result<u32>{r[key].as_u64().context("front input word absent")?.try_into().context("front input overflow")};
            let reply=Reply{backend:number("backend")?,interface:number("interface")?,offset:number("offset")?as usize,
                before:decode(&r["before"] )?,after:decode(&r["after"] )?,hresult:number("hresult")?};
            ensure!(self.backends.contains_key(&reply.backend)&&matches!(reply.offset,0xc|0x78)&&matches!(reply.before.len(),108|124)&&reply.before.len()==reply.after.len(),"front descriptor owner/extent differs");replies.push(reply);
        }
        ensure!(self.input==self.replies.len(),"front input binding precedes initial platform returns");
        self.replies.extend(replies);self.front=Some(Front{input:input_hash,surface:word(&window,0x70),started:false});self.validate()
    }
    pub(super) fn front_bound(&self)->bool{self.front.is_some()}
    pub(super) fn front_started(&self)->bool{self.front.as_ref().is_some_and(|f|f.started)}
    pub(super) fn front_complete(&self)->bool{self.front_started()&&self.complete()}
    pub(super) fn start_front(&mut self)->Result<()>{
        self.validate()?;ensure!(self.work.is_none()&&self.begin.as_ref().is_none_or(targets::Begin::complete)&&self.fields.active==0,"front acquisition overlaps a live frame");
        let front=self.front.as_mut().context("front inputs absent")?;ensure!(!front.started,"front acquisition already started");
        self.begin=Some(targets::Begin::new(front.surface).map_err(anyhow::Error::msg)?);front.started=true;self.validate()
    }
    pub(super) fn advance_front(&mut self,flags:u32)->Result<bool>{let surface=self.front.as_ref().context("front source absent")?.surface;self.advance(surface,flags)}
    pub(super) fn at_front_cut(&self,q:&serde_json::Value)->bool{self.front_started()&&self.at_native_cut(q)}
    pub(super) fn compare_front(&self,q:&serde_json::Value)->Result<()>{
        ensure!(self.at_front_cut(q),"front acquisition caller differs");
        self.compare(&serde_json::json!({"pc":q["pc"],"raw":q["raw"],"surface_state":q["front_acquire"]}))
    }
    pub(super) fn bind_copy(&mut self,source:&[u8],inputs:&[u8])->Result<()>{
        self.validate()?;
        if let Some(c)=&self.copy{ensure!(sims_fixtures::digest(source)==SOURCE&&sims_fixtures::digest(inputs)==c.input_source(),"copy input rebinding differs");return Ok(());}
        let c=copy::Copy::import(source,inputs)?;c.validate(&self.planes,&self.backends)?;self.copy=Some(c);self.validate()
    }
    pub(super) fn copy_bound(&self)->bool{self.copy.is_some()}
    pub(super) fn copy_started(&self)->bool{self.copy.as_ref().is_some_and(copy::Copy::started)}
    pub(super) fn copy_complete(&self)->bool{self.copy.as_ref().is_some_and(copy::Copy::complete)}
    pub(super) fn start_copy(&mut self,source:u32,rectangles:Vec<[i32;4]>)->Result<()>{
        self.copy.as_mut().context("copy inputs absent")?.start(source,rectangles)?;self.validate()
    }
    pub(super) fn advance_copy(&mut self,flags:u32)->Result<bool>{
        let before=self.clone();
        let result=(||{self.validate()?;let Some(c)=&mut self.copy else{return Ok(false)};
            let progress=c.advance(&mut self.planes,&self.backends,flags)?;self.validate()?;Ok(progress)})();
        if result.is_err(){*self=before;}result
    }
    pub(super) fn at_copy_cut(&self,q:&serde_json::Value)->bool{self.copy.as_ref().is_some_and(|c|c.at_native_cut(q,&self.planes))}
    pub(super) fn compare_copy(&self,q:&serde_json::Value)->Result<()>{
        ensure!(self.at_copy_cut(q),"surface copy caller differs");self.copy.as_ref().unwrap().compare_result(q)?;
        self.compare(&serde_json::json!({"pc":q["pc"],"raw":q["raw"],"surface_state":q["after_window_copy"]}))
    }
    pub(super) fn bind_end(&mut self,source:&[u8],native:&[u8])->Result<()>{
        self.validate()?;
        if self.end.is_some(){ensure!(sims_fixtures::digest(source)==SOURCE&&sims_fixtures::digest(native)=="c9a569590917da6cfbdb01a3da8469b11474bbb2ac8b4261bc77cce33819197a","end input rebinding differs");return self.validate();}
        let e=end::End::import(source,native)?;e.validate(&self.planes,&self.backends)?;self.end=Some(e);self.validate()
    }
    pub(super) fn bind_post_end(&mut self,source:&[u8],input:&[u8])->Result<()>{
        self.validate()?;ensure!(sims_fixtures::digest(source)==SOURCE&&matches!(sims_fixtures::digest(input).as_str(),END_STREAM|super::model::INPUT),"front EndFrame inputs differ");
        if let Some(e)=&self.post_end{ensure!(e.input_source()==sims_fixtures::digest(input),"front EndFrame input rebinding differs");return Ok(());}
        let e=end::End::import_platform(source,input)?;e.validate(&self.planes,&self.backends)?;self.post_end=Some(e);self.validate()
    }
    pub(super) fn post_end_bound(&self)->bool{self.post_end.is_some()}
    pub(super) fn coherent_post_stream(&self,input:&[u8])->bool{
        let h=sims_fixtures::digest(input);
        self.copy.as_ref().is_some_and(|c|c.input_source()==h)&&self.front.as_ref().is_some_and(|f|f.input==h)&&self.post_end.as_ref().is_some_and(|e|e.input_source()==h)
    }
    pub(super) fn post_end_started(&self)->bool{self.post_end.as_ref().is_some_and(end::End::started)}
    pub(super) fn post_end_complete(&self)->bool{self.post_end.as_ref().is_some_and(end::End::complete)}
    pub(super) fn start_post_end(&mut self)->Result<()>{
        self.validate()?;ensure!(self.work.is_none()&&self.front_complete(),"front EndFrame lost acquisition");
        self.post_end.as_mut().context("front EndFrame absent")?.start()?;self.validate()
    }
    pub(super) fn advance_post_end(&mut self,flags:u32)->Result<bool>{
        let before=self.clone();
        let result=(||{self.validate()?;let e=self.post_end.as_mut().context("front EndFrame absent")?;
            let progress=e.advance(&mut self.fields,&mut self.planes,&mut self.backends,flags)?;self.validate()?;Ok(progress)})();
        if result.is_err(){*self=before;}result
    }
    pub(super) fn at_post_end_cut(&self,q:&serde_json::Value)->bool{self.post_end.as_ref().is_some_and(|e|e.at_native_cut(q,&self.planes))}
    pub(super) fn compare_post_end(&self,q:&serde_json::Value)->Result<()>{
        ensure!(self.at_post_end_cut(q),"front EndFrame caller differs");
        self.compare(&serde_json::json!({"pc":q["pc"],"raw":q["post_front_end"]["window"],"surface_state":q["post_front_end"]}))
    }
    pub(super) fn acquired_here(&self)->Result<bool>{Ok(self.begin.as_ref().context("window acquisition owner absent")?.activated())}
    pub(super) fn end_overlay(&self)->Option<u8>{self.end.as_ref().map(end::End::overlay)}
    pub(super) fn end_started(&self)->bool{self.end.as_ref().is_some_and(end::End::started)}
    pub(super) fn end_complete(&self)->bool{self.end.as_ref().is_some_and(end::End::complete)}
    pub(super) fn end_consumed(&self)->Option<usize>{self.end.as_ref().map(end::End::consumed)}
    pub(super) fn start_end(&mut self)->Result<bool>{
        self.validate()?;let Some(e)=&mut self.end else{return Ok(false)};
        ensure!(self.work.is_none()&&self.begin.as_ref().is_some_and(targets::Begin::complete),"window end lost acquisition");
        e.start()?;self.validate()?;Ok(true)
    }
    pub(super) fn advance_end(&mut self,flags:u32)->Result<bool>{
        let before=self.clone();
        let result=(||{self.validate()?;let Some(e)=&mut self.end else{return Ok(false)};
            let progress=e.advance(&mut self.fields,&mut self.planes,&mut self.backends,flags)?;self.validate()?;Ok(progress)})();
        if result.is_err(){*self=before;}result
    }
    pub(super) fn at_end_cut(&self,q:&serde_json::Value)->bool{self.end.as_ref().is_some_and(|e|e.at_native_cut(q,&self.planes))}
    pub(super) fn compare_end(&self,q:&serde_json::Value)->Result<()>{
        self.validate()?;ensure!(self.at_end_cut(q),"end-frame caller differs");
        let row=serde_json::json!({"pc":q["pc"],"raw":q["surface_end"]["window"],"surface_state":q["surface_end"]});self.compare(&row)
    }
    pub(super) fn compare_window_tail(&self,q:&serde_json::Value)->Result<()>{
        if let Some(raw)=q["window_tail"].get("raw"){
            let raw=decode(raw)?;ensure!(raw.len()>=0x84&&raw[0x78]==self.fields.active&&word(&raw,0x7c)as i32==self.fields.phase&&word(&raw,0x80)==self.fields.surface,"window tail controls differ");
            if let Some(overlay)=self.end_overlay(){ensure!(raw[0x51]==overlay,"window tail overlay input differs");}
        }
        if let Some(bindings)=q.get("closed_plane_bindings"){ensure!(bindings["color"]==self.fields.buffers.color&&bindings["depth"]==self.fields.buffers.depth,"closed plane bindings differ");}
        Ok(())
    }
    pub(super) fn window_clip(&self)->[i16;4]{std::array::from_fn(|i|i16::from_le_bytes(self.middle[2*i..2*i+2].try_into().unwrap()))}
    pub(super) fn set_window_clip(&mut self,rectangle:[i32;4]){
        let packed=[rectangle[1]as i16,rectangle[0]as i16,rectangle[3]as i16,rectangle[2]as i16];
        for(i,v)in packed.iter().enumerate(){self.middle[2*i..2*i+2].copy_from_slice(&v.to_le_bytes());}
    }
    pub(super) fn set_window_clip_half(&mut self,half:usize,value:[i16;2])->Result<()>{
        ensure!(half<2,"clip half invalid");for(i,v)in value.iter().enumerate(){let at=half*4+i*2;self.middle[at..at+2].copy_from_slice(&v.to_le_bytes());}Ok(())
    }
    pub(super) fn compare_current_globals(&self,q:&serde_json::Value)->Result<()>{
        if let Some(v)=q["visible_geometry"].get("raster_globals"){
            let mut expected=self.fields.buffers.color_pitch.to_le_bytes().to_vec();expected.extend(&self.middle);expected.extend(self.fields.buffers.color.to_le_bytes());expected.extend(self.fields.buffers.depth.to_le_bytes());expected.extend(self.fields.buffers.depth_pitch.to_le_bytes());
            ensure!(decode(v)?==expected,"shared renderer target/clip globals differ");
        }Ok(())
    }
    pub fn import(encoded: &[u8], native: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE && sims_fixtures::digest(native) == TRACE,
            "surface source/witness differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "surface source epoch differs"
        );
        let m = Memory::new(&v)?;
        let mut planes = BTreeMap::new();
        let mut backends = BTreeMap::new();
        for r in v["paint"]["planes"]
            .as_array()
            .context("surface planes absent")?
        {
            let id = r["pointer"].as_u64().context("plane identity absent")? as u32;
            let raw = decode(&r["raw"])?;
            let backend = r["backend"].as_u64().context("backend identity absent")? as u32;
            let br = decode(&r["backend_raw"])?;
            ensure!(
                raw.len() == 0xd4
                    && word(&raw, 0x48) == backend
                    && m.read(id, raw.len())? == raw
                    && m.read(backend, br.len())? == br,
                "surface owner alias differs"
            );
            let vt = decode(&r["vtable_raw"])?;
            ensure!(
                m.read(word(&raw, 0), vt.len())? == vt
                    && word(&vt, 0x20) == 0x5dece2
                    && word(&vt, 0xcc) == 0x5df7a8
                    && word(&vt, 0xd0) == 0x5df7b2,
                "plane virtual methods differ"
            );
            let bt = decode(&r["backend_vtable_raw"])?;
            ensure!(
                m.read(word(&br, 0), bt.len())? == bt && word(&bt, 0xc) == 0x604e5e,
                "backend virtual method differs"
            );
            planes.insert(id, Plane { raw, backend });
            ensure!(
                backends
                    .insert(backend, backend::Fields { raw: br })
                    .is_none(),
                "surface backend alias requires a shared retained instance"
            );
        }
        let raw = m.read(0x09e88960, 0x520)?;
        let g = m.read(0x6f0c0c, 28)?;
        let fields = targets::Fields {
            active: raw[0x78],
            phase: word(&raw, 0x7c) as i32,
            surface: word(&raw, 0x80),
            buffers: targets::Buffers {
                color: word(&g, 16),
                color_pitch: word(&g, 0),
                depth: word(&g, 20),
                depth_pitch: word(&g, 24),
            },
        };
        use std::io::Read as _;
        let mut text = String::new();
        flate2::read::GzDecoder::new(native).read_to_string(&mut text)?;
        let rows = text
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let rows: Vec<_> = rows
            .iter()
            .filter(|r| r["kind"] == "world_drawing_setup" && !r["surface_state"].is_null())
            .collect();
        let mut replies = vec![];
        for pair in rows.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a["pc"] != 0x604eaau32 {
                continue;
            }
            ensure!(b["pc"] == 0x604eadu32, "DirectDraw return is not adjacent");
            let backend = a["registers"]["esi"]
                .as_u64()
                .context("DirectDraw owner absent")? as u32;
            ensure!(
                b["registers"]["esi"] == backend,
                "DirectDraw caller owner changed"
            );
            let call = &a["surface_state"]["directdraw"];
            let args: Vec<u32> = serde_json::from_value(call["args"].clone())?;
            ensure!(
                args.len() == 5 && args[1] == 0 && args[3..] == [1, 0],
                "DirectDraw argument profile differs"
            );
            let offset = args[2]
                .checked_sub(backend)
                .context("DirectDraw descriptor alias differs")? as usize;
            ensure!(
                matches!(offset, 0xc | 0x78),
                "DirectDraw descriptor kind differs"
            );
            let before = decode(&call["descriptor"])?;
            let after_row = b["surface_state"]["planes"]
                .as_array()
                .context("DirectDraw returned fields absent")?
                .iter()
                .find(|r| r["backend"] == backend)
                .context("DirectDraw returned owner absent")?;
            let ar = decode(&after_row["backend_raw"])?;
            let after = ar
                .get(offset..offset + before.len())
                .context("DirectDraw returned descriptor extent differs")?
                .to_vec();
            replies.push(Reply {
                backend,
                interface: args[0],
                offset,
                before,
                after,
                hresult: b["registers"]["eax"]
                    .as_u64()
                    .context("DirectDraw HRESULT absent")? as u32,
            });
        }
        ensure!(
            replies.len() == 2,
            "first target DirectDraw closure differs"
        );
        let s = Self {
            source: SOURCE.into(),
            trace: TRACE.into(),
            fields,
            middle: g[4..16].to_vec(),
            planes,
            backends,
            replies,
            input: 0,
            begin: None,
            work: None,
            end:None,
            copy:None,
            front:None,
            post_end:None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE
                && self.trace == TRACE
                && self.middle.len() == 12
                && self.input <= self.replies.len(),
            "surface source/input lifetime differs"
        );
        for p in self.planes.values() {
            ensure!(
                p.raw.len() == 0xd4
                    && word(&p.raw, 0x48) == p.backend
                    && self.backends.contains_key(&p.backend),
                "plane/backend owner differs"
            );
            let depth = word(&p.raw, 0x5c);
            ensure!(
                depth == 0 || self.planes.contains_key(&depth),
                "depth plane owner absent"
            );
        }
        for b in self.backends.values() {
            b.validate().map_err(anyhow::Error::msg)?;
        }
        if let Some(e)=&self.end{e.validate(&self.planes,&self.backends)?;}
        if let Some(c)=&self.copy{c.validate(&self.planes,&self.backends)?;}
        if let Some(f)=&self.front{ensure!((f.input==STREAM||f.input==END_STREAM||f.input==super::model::INPUT)&&self.planes.contains_key(&f.surface),"front input lifetime differs");}
        if let Some(e)=&self.post_end{e.validate(&self.planes,&self.backends)?;}
        if let Some(b) = &self.begin {
            b.validate().map_err(anyhow::Error::msg)?;
        } else {
            ensure!(self.work.is_none(), "surface work escaped its caller");
        }
        if let Some(Work::Lock {
            plane,
            frame: Some(f),
            ..
        }) = &self.work
        {
            f.validate(&self.backends[&self.planes[plane].backend])
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub fn start(&mut self, surface: u32) -> Result<()> {
        self.validate()?;
        ensure!(
            self.begin.is_none() && self.planes.contains_key(&surface),
            "target acquisition already started/unknown"
        );
        self.begin = Some(targets::Begin::new(surface).map_err(anyhow::Error::msg)?);
        self.validate()
    }
    pub fn complete(&self) -> bool {
        self.begin.as_ref().is_some_and(targets::Begin::complete)
    }
    pub fn consumed(&self) -> usize {
        self.input
    }
    fn depth(&self, surface: u32) -> Option<u32> {
        let p = word(&self.planes[&surface].raw, 0x5c);
        (p != 0).then_some(p)
    }
    fn publish_plane(&mut self, id: u32, flags: u32) -> Result<()> {
        let p = &self.planes[&id];
        let b = &self.backends[&p.backend];
        let mut fields = targets::Plane {
            buffer: word(&p.raw, 0x34),
            half_pitch: word(&p.raw, 0x38),
            locks: word(&p.raw, 0x3c),
            flags: word(&p.raw, 0x50),
        };
        ensure!(
            fields.lock(flags, 1, b.buffer(), b.pitch()) == 1,
            "successful plane lock failed"
        );
        let p = self.planes.get_mut(&id).unwrap();
        for (o, v) in [
            (0x34, fields.buffer),
            (0x38, fields.half_pitch),
            (0x50, fields.flags),
        ] {
            set(&mut p.raw, o, v);
        }
        Ok(())
    }
    pub fn advance(&mut self, surface: u32, flags: u32) -> Result<bool> {
        let before = self.clone();
        let result = self.advance_inner(surface, flags);
        if result.is_err() {
            *self = before;
        }
        result
    }
    fn advance_inner(&mut self, surface: u32, flags: u32) -> Result<bool> {
        self.validate()?;
        ensure!(self.begin.is_some(), "target acquisition not started");
        if let Some(work) = self.work.take() {
            match work {
                Work::Getter { plane, pitch } => {
                    let b = &self.backends[&self.planes[&plane].backend];
                    let reply = if pitch { b.pitch() } else { b.buffer() };
                    let request = self
                        .begin
                        .as_ref()
                        .unwrap()
                        .pending()
                        .context("target getter continuation absent")?;
                    self.begin
                        .as_mut()
                        .unwrap()
                        .acknowledge(request, reply)
                        .map_err(anyhow::Error::msg)?;
                }
                Work::Unlock { plane, flags } => {
                    ensure!(
                        !targets::Plane::needs_backend(flags),
                        "cleanup unlock requires a retained backend unlock child"
                    );
                    let p = self
                        .planes
                        .get_mut(&plane)
                        .context("cleanup plane absent")?;
                    let mut fields = targets::Plane {
                        buffer: word(&p.raw, 0x34),
                        half_pitch: word(&p.raw, 0x38),
                        locks: word(&p.raw, 0x3c),
                        flags: word(&p.raw, 0x50),
                    };
                    let result = fields.unlock(flags, 0);
                    for (o, v) in [
                        (0x34, fields.buffer),
                        (0x38, fields.half_pitch),
                        (0x3c, fields.locks),
                        (0x50, fields.flags),
                    ] {
                        set(&mut p.raw, o, v);
                    }
                    let req = self
                        .begin
                        .as_ref()
                        .unwrap()
                        .pending()
                        .context("cleanup continuation absent")?;
                    self.begin
                        .as_mut()
                        .unwrap()
                        .acknowledge(req, u32::from(result))
                        .map_err(anyhow::Error::msg)?;
                }
                Work::Lock {
                    plane,
                    flags,
                    stage,
                    mut frame,
                } => match stage {
                    LockStage::Entry => {
                        if targets::Plane::needs_backend(flags) {
                            frame = Some(backend::Lock::new());
                            self.work = Some(Work::Lock {
                                plane,
                                flags,
                                stage: LockStage::Backend,
                                frame,
                            });
                        } else {
                            self.publish_plane(plane, flags)?;
                            self.work = Some(Work::Lock {
                                plane,
                                flags,
                                stage: LockStage::Count,
                                frame,
                            });
                        }
                    }
                    LockStage::Backend => {
                        let id = self.planes[&plane].backend;
                        let b = self.backends.get_mut(&id).unwrap();
                        let f = frame.as_mut().context("backend continuation absent")?;
                        if let Some(request) = f.pending().cloned() {
                            let backend::Request::DirectDrawLock {
                                interface,
                                descriptor_offset,
                                descriptor,
                                flags: 1,
                                event: 0,
                            } = request
                            else {
                                self.work = Some(Work::Lock {
                                    plane,
                                    flags,
                                    stage,
                                    frame,
                                });
                                return Ok(false);
                            };
                            let r = self
                                .replies
                                .get(self.input)
                                .context("original DirectDraw input exhausted")?;
                            ensure!(
                                r.backend == id
                                    && r.interface == interface
                                    && r.offset == descriptor_offset
                                    && r.before == descriptor,
                                "original DirectDraw request differs"
                            );
                            f.directdraw_return(b, r.hresult, &r.after)
                                .map_err(anyhow::Error::msg)?;
                            self.input += 1;
                        } else {
                            ensure!(
                                f.advance(b).map_err(anyhow::Error::msg)?,
                                "backend lock stalled"
                            );
                        }
                        let stage = if f.complete() {
                            LockStage::Returned
                        } else {
                            LockStage::Backend
                        };
                        self.work = Some(Work::Lock {
                            plane,
                            flags,
                            stage,
                            frame,
                        });
                    }
                    LockStage::Returned => {
                        let reply = frame
                            .as_ref()
                            .context("backend return absent")?
                            .result()
                            .context("backend return value absent")?;
                        if reply == 0 {
                            let req = self
                                .begin
                                .as_ref()
                                .unwrap()
                                .pending()
                                .context("plane continuation absent")?;
                            self.begin
                                .as_mut()
                                .unwrap()
                                .acknowledge(req, 0)
                                .map_err(anyhow::Error::msg)?;
                        } else {
                            self.publish_plane(plane, flags)?;
                            self.work = Some(Work::Lock {
                                plane,
                                flags,
                                stage: LockStage::Count,
                                frame,
                            });
                        }
                    }
                    LockStage::Count => {
                        let p = self.planes.get_mut(&plane).unwrap();
                        let count = word(&p.raw, 0x3c).wrapping_add(1);
                        set(&mut p.raw, 0x3c, count);
                        let req = self
                            .begin
                            .as_ref()
                            .unwrap()
                            .pending()
                            .context("plane continuation absent")?;
                        self.begin
                            .as_mut()
                            .unwrap()
                            .acknowledge(req, 1)
                            .map_err(anyhow::Error::msg)?;
                    }
                },
            }
        } else if let Some(req) = self.begin.as_ref().unwrap().pending() {
            self.work = Some(match req {
                targets::Request::Lock { surface, flags } => Work::Lock {
                    plane: surface,
                    flags,
                    stage: LockStage::Entry,
                    frame: None,
                },
                targets::Request::Unlock { surface, flags } => Work::Unlock {
                    plane: surface,
                    flags,
                },
                targets::Request::Buffer { surface } => Work::Getter {
                    plane: surface,
                    pitch: false,
                },
                targets::Request::Pitch { surface } => Work::Getter {
                    plane: surface,
                    pitch: true,
                },
            });
        } else {
            let depth = self.depth(surface);
            return self
                .begin
                .as_mut()
                .unwrap()
                .advance(&mut self.fields, flags, depth)
                .map_err(anyhow::Error::msg);
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, row: &serde_json::Value) -> bool {
        let pc = row["pc"].as_u64().unwrap_or(0) as u32;
        if self.begin.as_ref().is_some_and(|f| f.at_native_cut(pc)) && self.work.is_none() {
            return true;
        }
        match &self.work {
            Some(Work::Lock {
                plane,
                stage,
                frame,
                ..
            }) => match pc {
                0x5dece2 => *stage == LockStage::Entry && row["registers"]["ecx"] == *plane,
                0x5ded10 => *stage == LockStage::Returned,
                0x5ded2e => *stage == LockStage::Count,
                _ => {
                    *stage == LockStage::Backend
                        && frame.as_ref().is_some_and(|f| f.at_native_cut(pc))
                }
            },
            Some(Work::Getter { plane, pitch }) => {
                row["registers"]["ecx"] == *plane && pc == if *pitch { 0x5df7b2 } else { 0x5df7a8 }
            }
            _ => false,
        }
    }
    pub fn compare(&self, row: &serde_json::Value) -> Result<()> {
        self.validate()?;
        let s = &row["surface_state"];
        ensure!(!s.is_null(), "native surface observation absent");
        let raw = decode(&row["raw"])?;
        ensure!(
            raw[0x78] == self.fields.active
                && word(&raw, 0x7c) as i32 == self.fields.phase
                && word(&raw, 0x80) == self.fields.surface,
            "target control fields differ"
        );
        let mut globals = self.fields.buffers.color_pitch.to_le_bytes().to_vec();
        globals.extend(&self.middle);
        globals.extend(self.fields.buffers.color.to_le_bytes());
        globals.extend(self.fields.buffers.depth.to_le_bytes());
        globals.extend(self.fields.buffers.depth_pitch.to_le_bytes());
        ensure!(
            decode(&s["globals"])? == globals,
            "target global fields differ"
        );
        for r in s["planes"]
            .as_array()
            .context("native plane states absent")?
        {
            let id = r["pointer"].as_u64().context("native plane id absent")? as u32;
            let p = self.planes.get(&id).context("native plane not retained")?;
            ensure!(
                p.raw == decode(&r["raw"])?
                    && self.backends[&p.backend].raw == decode(&r["backend_raw"])?,
                "surface/plane/backend fields differ at{:x} plane{id:x}",
                row["pc"].as_u64().unwrap_or(0)
            );
        }
        Ok(())
    }
}
