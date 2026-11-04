//!4a9262's retained entry and4aceb8's initial window/region work. Surface
//! acquisition is an explicit child; this does not acknowledge PaintWorld.
use super::*;
use anyhow::Context as _;
mod surfaces;
mod background;
mod tiles;
mod post;
pub(super) mod model;
mod drawable;
pub mod row;
type GeometryView<'a> = Option<(&'a row::State,&'a sims_vm::scheduler::clock::draw::context::Frame)>;
const SOURCE: &str = "0ce7ea3cb00886c6f9ead5e36d279c8ac88f80ea65462d456148e4f67b753909";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Marker,
    Append,
    Marked,
    Window,
    Clipped,
    Flushed,
    WindowReturned,
    Acquiring,
    Acquired,
    WindowBound,
    Background,
    AfterBackground,
    Tiles,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Timestamp,
    FlushSurfaces { renderer: u32 },
    BeginFrame { surface: u32 },
    ActiveFrame,
    ContinuePaint,
    WindowContents,
    WindowTiles,
    BackgroundChild { request: sims_vm::scheduler::clock::draw::background::Request },
    TileRow { request: sims_sim::lot_presenter::tile_scan::Request },
    ContinueWindowLevels,
    LevelGeometry { model: u32, rectangle: [i32; 4], level: i32 },
    LevelGeometryChild { function: u32, model: u32, rectangle: [i32; 4], level: i32 },
    CopyRenderedRegions { source:u32,count:usize },
    DrawObject { object:sims_vm::ObjectId,pointer:u32 },
    DrawableChild { object:sims_vm::ObjectId,pointer:u32,function:u32 },
}
/// Immutable inputs needed by the computed geometry continuation. This query
/// never consumes an observation or advances the caller/child state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryInput { Iterator, First, Corner, Segments, Visible, Materials { key: u32 } }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    stage: Stage,
    viewport: [i32; 4],
    surface: u32,
    available: u32,
    active_frame: u8,
    renderer: u32,
    flush: u8,
    record: Option<Record>,
    #[serde(default)]
    surfaces: Option<surfaces::State>,
    #[serde(default)]
    background: Option<background::State>,
    #[serde(default)]
    tiles: Option<tiles::State>,
    #[serde(default)]
    row: Option<row::State>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    post:Option<post::Cursor>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    drawable:Option<drawable::Cursor>,
}
impl State {
    pub fn geometry_input_request(&self) -> Option<GeometryInput> {
        self.tiles.as_ref().and_then(tiles::State::geometry_input_request)
    }
    fn import(encoded: &[u8], drawing: &drawing::State) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "paint source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "paint source epoch differs"
        );
        let m = Memory::new(&v)?;
        let raw = m.read(0x09e88960, 0x520)?;
        let surface = word(&raw, 0x70 + if drawing.camera_mode() == 0 { 4 } else { 0 });
        let renderer = word(&m.read(word(&m.read(0x6e48bc, 4)?, 0) + 0x20, 4)?, 0);
        let state = Self {
            source: SOURCE.into(),
            stage: Stage::Entry,
            viewport: std::array::from_fn(|i| word(&raw, 0x28 + 4 * i) as i32),
            surface,
            available: if surface != 0 {
                word(&m.read(surface, 0xd4)?, 0x5c)
            } else {
                0
            },
            active_frame: raw[0x78],
            renderer,
            flush: m.read(renderer, 0x24)?[0x1e],
            record: None,
            surfaces: None,
            background: None,
            tiles: None,
            row: None,
            post:None,
            drawable:None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(s) = &self.surfaces {
            s.validate()?;
        }
        if let Some(b) = &self.background { b.validate()?; }
        if let Some(t) = &self.tiles { t.validate()?; }
        if let Some(r) = &self.row {r.validate()?;ensure!((self.stage==Stage::Tiles||self.stage==Stage::WindowReturned&&r.acknowledged())&&(r.acknowledged()||self.tiles.as_ref().and_then(|t|t.row())==Some(r.request())),"row lost its actual scan callback");}
        if let Some(r)=self.row.as_ref().filter(|r|!r.acknowledged()){if let Some(second)=r.ordinary_second(){ensure!(self.tiles.as_ref().and_then(|t|t.row_second())==Some(second),"ordinary row lost its scanner half");}}
        ensure!(self.stage!=Stage::Tiles||self.tiles.is_some(),"paint lost its tile continuation");
        ensure!(!matches!(self.stage,Stage::Background|Stage::AfterBackground)||self.background.is_some(),"paint lost its background continuation");
        ensure!(
            !matches!(
                self.stage,
                Stage::Acquiring | Stage::Acquired | Stage::WindowBound | Stage::Background | Stage::AfterBackground | Stage::Tiles
            ) || self.surfaces.is_some(),
            "paint lost its surface continuation"
        );
        ensure!(
            self.source == SOURCE && self.renderer != 0,
            "paint input owner differs"
        );
        ensure!(
            self.record.is_some() == !matches!(self.stage, Stage::Entry | Stage::Marker),
            "paint marker lifetime differs"
        );
        Ok(())
    }
    pub fn pending(&self) -> Option<Request> {
        if self.timestamp_name().is_some(){return Some(Request::Timestamp);}
        match self.stage {
            Stage::Marker => Some(Request::Timestamp),
            Stage::Clipped if self.flush != 0 => Some(Request::FlushSurfaces {
                renderer: self.renderer,
            }),
            Stage::Flushed if self.active_frame == 0 => Some(Request::BeginFrame {
                surface: self.surface,
            }),
            Stage::Flushed => Some(Request::ActiveFrame),
            Stage::WindowReturned => Some(Request::ContinuePaint),
            Stage::WindowBound => Some(Request::WindowContents),
            Stage::Background => self.background.as_ref().and_then(|b|b.pending().cloned()).map(|request|Request::BackgroundChild{request}),
            Stage::AfterBackground => Some(Request::WindowTiles),
            Stage::Tiles => Some(self.tiles.as_ref().and_then(|t|t.geometry_request()).unwrap_or_else(||self.tiles.as_ref().and_then(|t|t.row().cloned()).map_or(Request::ContinueWindowLevels,|request|Request::TileRow{request}))),
            _ => None,
        }
    }
    pub(super) fn has_record(&self) -> bool {
        self.record.is_some()
    }
    pub(super) fn record_name(&self)->Option<u32>{self.record.map(|r|r.name)}
    pub(super) fn validate_post(&self,drawing:Option<&drawing::State>,context:Option<&sims_vm::scheduler::clock::draw::context::Frame>)->Result<()>{
        if let Some(p)=&self.post{ensure!(self.stage==Stage::WindowReturned,"post-window child lost paint");p.validate(drawing.context("post-window drawing owner absent")?)?;}
        if let Some(child)=&self.drawable{
            let Some(Request::DrawObject{object,pointer})=self.post.as_ref().and_then(|p|p.drawable(drawing?)) else{anyhow::bail!("drawable lost its pending call")};
            ensure!(child.identity()==(object,pointer),"drawable caller identity differs");
            child.validate(context.context("drawable lost its shared drawing context")?)?;
        }
        Ok(())
    }
    pub(super) fn timestamp_name(&self)->Option<u32>{if self.stage==Stage::Marker{Some(0x6c735c)}else{self.post.as_ref().and_then(|p|p.clock().map(|(_,name)|name))}}
    pub fn post_clock_request(&self)->Option<(usize,u32)>{self.post.as_ref().and_then(post::Cursor::clock)}
    pub fn at_post_window_cut(&self,pc:u32)->bool{self.drawable.is_none()&&self.stage==Stage::WindowReturned&&self.post.as_ref().is_some_and(|p|p.pc()==pc)}
    pub fn at_surface_copy_cut(&self,q:&serde_json::Value)->bool{self.at_post_window_cut(0x4a934c)&&self.surfaces.as_ref().is_some_and(|s|s.at_copy_cut(q))}
    pub fn at_front_surface_cut(&self,q:&serde_json::Value)->bool{self.post.as_ref().is_some_and(post::Cursor::front_pending)&&self.surfaces.as_ref().is_some_and(|s|s.at_front_cut(q))}
    pub fn at_front_end_cut(&self,q:&serde_json::Value)->bool{self.post.as_ref().is_some_and(post::Cursor::end_pending)&&self.surfaces.as_ref().is_some_and(|s|s.at_post_end_cut(q))}
    pub(super) fn at_entry(&self) -> bool {
        self.stage == Stage::Entry
    }
    pub(super) fn timestamp(&mut self, time: u64) -> Result<Record> {
        if let Some(p)=self.post.as_mut().filter(|p|p.clock().is_some()){
            let name=p.clock().unwrap().1;let r=Record{timestamp:time,name,argument:0};p.clock_return()?;self.record=Some(r);self.validate()?;return Ok(r);
        }
        ensure!(
            self.stage == Stage::Marker,
            "paint timestamp is not pending"
        );
        let record = Record {
            timestamp: time,
            name: 0x6c735c,
            argument: 0,
        };
        self.record = Some(record);
        self.stage = Stage::Append;
        self.validate()?;
        Ok(record)
    }
    fn advance(
        &mut self,
        drawing: &mut drawing::State,
        scene: &mut scene::State,
        profiler: &mut Buffer,
        flags: u32,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Entry => self.stage = Stage::Marker,
            Stage::Marker => return Ok(false),
            Stage::Append => {
                profiler
                    .append(self.record.unwrap())
                    .map_err(anyhow::Error::msg)?;
                self.stage = Stage::Marked;
            }
            Stage::Marked => self.stage = Stage::Window,
            Stage::Window => {
                if scene.regions.len() == 0 || self.surface == 0 || self.available == 0 {
                    self.stage = Stage::WindowReturned;
                } else {
                    let (enabled, weight) = drawing.region_policy();
                    if !scene
                        .regions
                        .clip(self.viewport, enabled, weight)
                        .map_err(anyhow::Error::msg)?
                    {
                        return Ok(false);
                    }
                    self.stage = Stage::Clipped;
                }
            }
            Stage::Clipped => {
                if self.flush != 0 {
                    return Ok(false);
                }
                self.stage = Stage::Flushed;
            }
            Stage::Flushed => {
                let Some(s) = self.surfaces.as_mut() else {
                    return Ok(false);
                };
                ensure!(
                    self.active_frame == 0,
                    "existing active frame needs its retained caller"
                );
                s.start(self.surface)?;
                self.stage = Stage::Acquiring;
            }
            Stage::Acquiring => {
                let s = self.surfaces.as_mut().context("paint surfaces absent")?;
                if !s.advance(self.surface, flags)? {
                    return Ok(false);
                }
                if s.complete() {
                    self.stage = Stage::Acquired;
                }
            }
            Stage::Acquired => {
                let s = self.surfaces.as_mut().context("paint surfaces absent")?;
                if s.fields.active != 0 {
                    s.fields.phase = i32::from(drawing.camera_mode() == 0);
                }
                self.stage = Stage::WindowBound;
            }
            Stage::WindowBound | Stage::Background => {
                let Some(b)=self.background.as_mut()else{return Ok(false)};
                if !b.advance(&scene.regions,self.viewport,flags)? { return Ok(false); }
                self.stage=if b.complete(){Stage::AfterBackground}else{Stage::Background};
            }
            Stage::AfterBackground | Stage::Tiles => {
                let Some(t)=self.tiles.as_mut()else{return Ok(false)};
                if !t.advance(&scene.regions,self.surfaces.as_mut().context("tile surfaces absent")?,flags)?{return Ok(false);}
                self.stage=if t.complete(){Stage::WindowReturned}else{Stage::Tiles};
            }
            Stage::WindowReturned => {
                let Some(p)=&mut self.post else{return Ok(false)};
                if let Some((source,count))=p.copy(drawing){
                    let s=self.surfaces.as_mut().context("copy surface state absent")?;
                    if !s.copy_bound(){return Ok(false);}
                    if !s.copy_started(){
                        let rectangles=(0..count).map(|i|drawing.model_regions().rectangle(i).map_err(anyhow::Error::msg)).collect::<Result<Vec<_>>>()?;
                        s.start_copy(source,rectangles)?;
                    }else if !s.advance_copy(flags)?{return Ok(false);}
                    if s.copy_complete(){p.copy_return()?;}
                }else if p.front_pending(){
                    let s=self.surfaces.as_mut().context("front surface state absent")?;
                    if !s.front_bound(){return Ok(false);}
                    if !s.front_started(){s.start_front()?;}
                    else if s.front_complete(){p.front_return()?;}
                    else if !s.advance_front(flags)?{return Ok(false);}
                }else if p.end_pending(){
                    let s=self.surfaces.as_mut().context("front EndFrame surface state absent")?;
                    if !s.post_end_bound(){return Ok(false);}
                    if !s.post_end_started(){s.start_post_end()?;}
                    else if s.post_end_complete(){p.end_return()?;}
                    else if !s.advance_post_end(flags)?{return Ok(false);}
                }else if let Some(begin)=p.model_pending(){
                    let Some(m)=drawing.model_mut()else{return Ok(false)};
                    if !m.started(){m.start(begin,1)?;}
                    else if m.complete(){p.model_return(m.finish()?)?;}
                    else if !m.advance()?{return Ok(false);}
                }else if !p.advance(drawing,scene,profiler,self.record,flags,self.viewport,self.surfaces.as_mut().context("post-window surfaces absent")?)?{return Ok(false);}
            },
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4a9262 => self.stage == Stage::Entry,
            0x4a9297 => self.stage == Stage::Marked,
            0x4aceb8 => self.stage == Stage::Window,
            0x4acf1c => self.stage == Stage::Clipped,
            0x4acf29 => self.stage == Stage::Flushed,
            _ => false,
        }
    }
    pub fn at_surface_cut(&self, row: &serde_json::Value) -> bool {
        let pc = row["pc"].as_u64().unwrap_or(0) as u32;
        match pc {
            0x4acf29 => self.stage == Stage::Flushed,
            0x4b0276 | 0x4acf45 => self.stage == Stage::Acquired,
            0x4acf4e => self.stage == Stage::WindowBound,
            _ => {
                self.stage == Stage::Acquiring
                    && self.surfaces.as_ref().is_some_and(|s| s.at_native_cut(row))
            }
        }
    }
    pub fn surface_inputs_consumed(&self) -> Option<usize> {
        self.surfaces.as_ref().map(surfaces::State::consumed)
    }
    pub fn surface_end_inputs_consumed(&self)->Option<usize>{self.surfaces.as_ref().and_then(surfaces::State::end_consumed)}
    pub fn at_surface_end_cut(&self,row:&serde_json::Value)->bool{self.stage==Stage::Tiles&&self.surfaces.as_ref().is_some_and(|s|s.at_end_cut(row))}
    pub fn tile_row(&self)->Option<&sims_sim::lot_presenter::tile_scan::Request>{self.tiles.as_ref().and_then(|t|t.row())}
    pub fn row_prefix(&self)->Option<&row::State>{self.row.as_ref()}
    pub fn at_background_cut(&self,row:&serde_json::Value)->bool {
        let pc=row["pc"].as_u64().unwrap_or(0)as u32;
        matches!(self.stage,Stage::WindowBound|Stage::Background|Stage::AfterBackground)&&self.background.as_ref().is_some_and(|b|b.at_native_cut(pc))
    }
    pub fn at_tile_cut(&self,row:&serde_json::Value)->bool{
        let pc=row["pc"].as_u64().unwrap_or(0)as u32;
        matches!(self.stage,Stage::AfterBackground|Stage::Tiles|Stage::WindowReturned)&&self.tiles.as_ref().is_some_and(|t|t.at_native_cut(pc))
    }
    pub fn tile_binding_allowed(&self)->bool{self.at_entry()||self.stage==Stage::AfterBackground}
    fn compare(
        &self,
        row: &serde_json::Value,
        scene: &scene::State,
        profiler: &Buffer,
        scratch: Option<[u32; 4]>,
    ) -> Result<()> {
        let pc = row["pc"].as_u64().context("paint PC absent")? as u32;
        ensure!(
            row["kind"] == "world_drawing_setup" && row["tick"] == 89081 && self.at_native_cut(pc),
            "paint stage differs at{pc:x}"
        );
        let decode = |v: &serde_json::Value| {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("paint bytes absent")?,
            )
        };
        let raw = decode(&row["raw"])?;
        ensure!(
            std::array::from_fn::<_, 4, _>(|i| word(&raw, 0x28 + 4 * i) as i32) == self.viewport
                && raw[0x78] == self.active_frame,
            "paint view inputs differ"
        );
        let owner = &row["paint_owners"];
        ensure!(
            owner["renderer"] == self.renderer
                && decode(&owner["renderer_raw"])?[0x1e] == self.flush,
            "paint renderer owner differs"
        );
        let regions = row["lists"]
            .as_array()
            .context("paint region owners absent")?
            .iter()
            .find(|r| r["label"] == "world")
            .context("paint World regions absent")?;
        ensure!(
            scene.regions.header == decode(&regions["raw"])?
                && scene.regions.backing == decode(&regions["backing"])?,
            "paint shared region bytes differ at{pc:x}"
        );
        if let Some(record) = self.record {
            ensure!(
                scratch == Some(record.words()) && profiler.records().last() == Some(&record),
                "paint retained marker differs"
            );
            let native = decode(&row["profiler"]["used_raw"])?;
            let words: Vec<_> = native[native.len() - 16..]
                .chunks_exact(4)
                .map(|b| word(b, 0))
                .collect();
            ensure!(words == record.words(), "paint original marker differs");
        }
        Ok(())
    }
}
impl Owner {
    pub(in crate::original_session::draw_parent) fn drawing_excluded_person(&self)->Option<u32>{self.drawing.as_ref()?.excluded_person()}
    pub(in crate::original_session::draw_parent) fn bind_surface_post_end(&mut self,source:&[u8],input:&[u8])->Result<()>{
        self.validate()?;let p=self.paint.as_mut().context("paint absent")?;ensure!(p.stage==Stage::WindowReturned&&p.post.is_some(),"front EndFrame binding stage differs");
        p.surfaces.as_mut().context("surface state absent")?.bind_post_end(source,input)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn bind_surface_front(&mut self,source:&[u8],input:&[u8])->Result<()>{
        self.validate()?;let p=self.paint.as_mut().context("paint absent")?;ensure!(p.stage==Stage::WindowReturned&&p.post.is_some(),"front binding stage differs");
        p.surfaces.as_mut().context("surface state absent")?.bind_front(source,input)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn bind_surface_copy(&mut self,source:&[u8],inputs:&[u8])->Result<()>{
        self.validate()?;let p=self.paint.as_mut().context("paint absent")?;
        ensure!(p.stage==Stage::WindowReturned&&p.post.is_some(),"surface copy binding stage differs");
        p.surfaces.as_mut().context("copy surface state absent")?.bind_copy(source,inputs)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn bind_post_window(&mut self,source:&[u8])->Result<()>{
        self.validate()?;let p=self.paint.as_mut().context("paint absent")?;ensure!(p.stage==Stage::WindowReturned&&p.post.is_none(),"post-window binding stage differs");
        let c=post::Cursor::import(source)?;c.validate(self.drawing.as_ref().context("drawing absent")?)?;p.post=Some(c);self.validate()
    }
    pub(in crate::original_session::draw_parent) fn bind_model(&mut self,source:&[u8],input:&[u8],classify:&classification::State)->Result<()>{
        self.validate()?;ensure!(self.paint.as_ref().is_some_and(|p|p.stage==Stage::WindowReturned&&p.post.is_some()),"model binding lost PaintWorld");
        ensure!(self.paint.as_ref().and_then(|p|p.surfaces.as_ref()).is_some_and(|s|s.coherent_post_stream(input)),"model and surface platform streams differ");
        self.drawing.as_mut().context("model drawing owner absent")?.bind_model(source,input,classify)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn at_model_cut(&self,q:&serde_json::Value)->bool{
        self.paint.as_ref().and_then(|p|p.post.as_ref()).is_some_and(|p|p.model_pending().is_some())&&self.drawing.as_ref().and_then(drawing::State::model).is_some_and(|m|m.at_cut(q))
    }
    pub(in crate::original_session::draw_parent) fn post_window_request(&self)->Option<Request>{
        let paint=self.paint.as_ref()?;
        if let Some(child)=&paint.drawable{return child.pending().or_else(||{
            let(object,pointer)=child.identity();Some(Request::DrawObject{object,pointer})
        });}
        let p=paint.post.as_ref()?;let d=self.drawing.as_ref()?;
        if let Some((source,count))=p.copy(d){return Some(Request::CopyRenderedRegions{source,count});}
        p.drawable(d)
    }
    pub(in crate::original_session::draw_parent) fn at_drawable_cut(&self,q:&serde_json::Value)->bool{
        self.paint.as_ref().and_then(|p|p.drawable.as_ref()).is_some_and(|p|q["pc"]==p.pc())
    }
    pub(in crate::original_session::draw_parent) fn advance_drawable(&mut self,sim:&sims_sim::Simulation,view:&composition::View)->Result<bool>{
        self.validate()?;
        let paint=self.paint.as_mut().context("drawable paint owner absent")?;
        let request=paint.post.as_ref().context("drawable parent absent")?.drawable(self.drawing.as_ref().context("drawable drawing owner absent")?);
        let Some(Request::DrawObject{object,pointer})=request else{anyhow::bail!("drawable call is not pending")};
        if paint.drawable.is_none(){paint.drawable=Some(drawable::Cursor::new(object,pointer)?);return Ok(true);}
        let child=paint.drawable.as_mut().unwrap();
        ensure!(child.identity()==(object,pointer),"drawable lost its caller");
        let progress=child.advance(sim,view,self.context.as_mut().context("drawable shared context absent")?)?;
        self.validate()?;Ok(progress)
    }
    pub(in crate::original_session::draw_parent) fn compare_post_window(&self,q:&serde_json::Value,profiler:&Buffer)->Result<()>{
        self.validate()?;let p=self.paint.as_ref().context("paint absent")?;
        let post=p.post.as_ref().context("post-window absent")?;let drawing=self.drawing.as_ref().context("drawing absent")?;
        if self.at_drawable_cut(q){
            p.drawable.as_ref().unwrap().compare(q,self.context.as_ref().context("drawable shared context absent")?)?;
            post.compare_owned(q,drawing,profiler,p.record)?;
        }else if self.at_model_cut(q){
            post.compare_owned(q,drawing,profiler,p.record)?;
        }else if p.at_front_end_cut(q){
            p.surfaces.as_ref().unwrap().compare_post_end(q)?;post.compare_owned(q,drawing,profiler,p.record)?;
        }else if p.at_front_surface_cut(q){
            p.surfaces.as_ref().unwrap().compare_front(q)?;post.compare_owned(q,drawing,profiler,p.record)?;
        }else if p.at_surface_copy_cut(q){
            p.surfaces.as_ref().unwrap().compare_copy(q)?;post.compare_owned(q,drawing,profiler,p.record)?;
        }else{post.compare(q,drawing,profiler,p.record)?;}
        let r=serde_json::json!({"window_tail":{"raw":q["raw"]}});p.surfaces.as_ref().context("surfaces absent")?.compare_window_tail(&r)
    }
    pub(in crate::original_session::draw_parent) fn bind_paint(
        &mut self,
        source: &[u8],
    ) -> Result<()> {
        ensure!(
            self.at_entry() && self.paint.is_none(),
            "paint binding already executing"
        );
        self.paint = Some(State::import(
            source,
            self.drawing
                .as_ref()
                .context("paint drawing source absent")?,
        )?);
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn paint(&self) -> Option<&State> {
        self.paint.as_ref()
    }
    pub(in crate::original_session::draw_parent) fn advance_paint(
        &mut self,
        scene: &mut scene::State,
        profiler: &mut Buffer,
    ) -> Result<bool> {
        ensure!(
            self.phase == Phase::Painting,
            "paint prefix does not own World"
        );
        let flags = self
            .context
            .as_ref()
            .context("paint context absent")?
            .state()
            .flags(0)
            .map_err(anyhow::Error::msg)?;
        let progress = self
            .paint
            .as_mut()
            .context("paint source absent")?
            .advance(
                self.drawing
                    .as_mut()
                    .context("paint drawing source absent")?,
                scene,
                profiler,
                flags,
            )?;
        self.validate()?;
        Ok(progress)
    }
    pub(in crate::original_session::draw_parent) fn bind_region_loop(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().context("paint absent")?.tiles.as_mut().context("tiles absent")?.bind_region_loop(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_tiles(&mut self,source:&[u8])->Result<()>{
        let p=self.paint.as_mut().context("paint absent")?;ensure!(p.tile_binding_allowed()&&p.tiles.is_none(),"tile binding already executing");
        p.tiles=Some(tiles::State::import(source)?);self.validate()
    }
    pub(in crate::original_session::draw_parent) fn bind_row(&mut self,source:&[u8])->Result<()>{
        let p=self.paint.as_mut().context("paint absent")?;ensure!(p.stage==Stage::Tiles&&p.row.as_ref().is_none_or(|r|r.acknowledged()),"row binding escaped pending callback");
        let request=p.tiles.as_ref().and_then(|t|t.row()).context("row callback absent")?.clone();
        let second=p.tiles.as_ref().and_then(|t|t.row_second()).context("row caller scan phase absent")?;
        if let Some(r)=&mut p.row{r.restart(source,request)?;}else{p.row=Some(row::State::import(source,request)?);}p.row.as_mut().unwrap().set_second(second)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn advance_row(&mut self,simulation:&sims_sim::Simulation)->Result<bool>{self.advance_row_objects(simulation,None)}
    pub(in crate::original_session::draw_parent) fn advance_row_objects(&mut self,simulation:&sims_sim::Simulation,classify:Option<&mut classification::State>)->Result<bool>{
        self.validate()?;
        let p=self.paint.as_mut().context("paint absent")?;let r=p.row.as_mut().context("row prefix absent")?;
        let progress=r.advance(simulation,self.context.as_mut().context("shared context absent")?,p.surfaces.as_ref().context("retained paint surfaces absent")?.window_clip(),classify)?;
        self.validate()?;Ok(progress)
    }
    pub(in crate::original_session::draw_parent) fn compare_row(&self,row:&serde_json::Value)->Result<()>{
        self.paint.as_ref().and_then(|p|p.row.as_ref()).context("row prefix absent")?.compare(row,self.context.as_ref().context("shared context absent")?)
    }
    pub(in crate::original_session::draw_parent) fn compare_shared_graphics(&self,row:&serde_json::Value)->Result<()>{
        self.paint.as_ref().and_then(|p|p.surfaces.as_ref()).context("surfaces absent")?.compare_current_globals(row)?;
        self.paint.as_ref().and_then(|p|p.row.as_ref()).context("row prefix absent")?.compare_shared(row,self.context.as_ref().context("shared context absent")?)
    }
    pub(in crate::original_session::draw_parent) fn bind_level_geometry(&mut self,source:&[u8])->Result<()>{
        self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("tiles absent")?.bind_level_geometry(source)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn bind_geometry_body(&mut self,source:&[u8])->Result<()>{
        self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("tiles absent")?.bind_geometry_body(source)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn bind_geometry_child(&mut self,source:&[u8])->Result<()>{
        self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("tiles absent")?.bind_geometry_child(source)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn advance_geometry_child(&mut self,sim:&sims_sim::Simulation,controls:Option<&sims_vm::interface::state::State>,graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>)->Result<bool>{
        self.validate()?;let paint=self.paint.as_mut().context("paint absent")?;let p=paint.tiles.as_mut().context("tiles absent")?.advance_geometry_child(sim,self.context.as_mut().context("geometry drawing context absent")?,paint.row.as_mut(),controls,graph)?;
        if let Some((half,value))=paint.row.as_mut().and_then(|r|r.take_geometry_clip()){paint.surfaces.as_mut().context("clip publication lost surfaces")?.set_window_clip_half(half,value)?;}
        self.validate()?;Ok(p)
    }
    pub(in crate::original_session::draw_parent) fn bind_geometry_catalog(&mut self,meta:&[u8],raw:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("catalog row absent")?.bind_geometry_catalog(meta,raw)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_diagonal_columns(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().context("paint absent")?.row.as_mut().context("row absent")?.bind_geometry_diagonal_columns(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_raster(&mut self,source:&[u8],material:&[u8])->Result<()>{let p=self.paint.as_mut().context("paint absent")?;let targets=p.surfaces.as_ref().context("surfaces absent")?.fields.buffers;p.row.as_mut().context("row absent")?.bind_geometry_raster(source,material,targets)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_side(&mut self,source:&[u8],material:&[u8])->Result<()>{let p=self.paint.as_mut().context("paint absent")?;p.row.as_mut().context("row absent")?.bind_geometry_side(source,material)?;p.tiles.as_mut().context("tiles absent")?.bind_geometry_side()?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_side_alt(&mut self,input:&[u8],color:&[u8],mask:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("alternate side row absent")?.bind_geometry_side_alt(input,color,mask)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn start_geometry_last_bound_side_if_pending(&mut self)->Result<bool>{let bound=self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("side tiles absent")?.try_bind_geometry_last_side()?;self.validate()?;Ok(bound)}
    pub(in crate::original_session::draw_parent) fn start_geometry_bound_side(&mut self)->Result<()>{self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("side tiles absent")?.bind_geometry_side()?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_visible(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("tiles absent")?.bind_geometry_visible(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_materials(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("tiles absent")?.bind_geometry_materials(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_corner(&mut self,source:&[u8],setup:&[u8],graph:&sims_sim::lot_presenter::walls::construction::Graph)->Result<()>{self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("tiles absent")?.bind_geometry_corner(source,setup,graph)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_geometry_segments(&mut self,source:&[u8],graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>)->Result<()> {self.paint.as_mut().and_then(|p|p.tiles.as_mut()).context("tiles absent")?.bind_geometry_segments(source,graph)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_raster(&mut self,source:&[u8],color:&[u8],depth:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_raster(source,color,depth)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn extend_row_depth(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.extend_depth_rows(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_floor_sprite(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_floor_sprite(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_shadow(&mut self,encoded:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_shadow(encoded)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_raster(&mut self,encoded:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_raster(encoded)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_raster_mirror(&mut self,encoded:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_raster_mirror(encoded)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_raster_batch(&mut self,encoded:&[u8])->Result<()>{let p=self.paint.as_mut().context("paint absent")?;let targets=p.surfaces.as_ref().context("surfaces absent")?.fields.buffers;p.row.as_mut().context("row absent")?.bind_object_raster_batch(encoded,targets)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_catalog(&mut self,encoded:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_catalog(encoded,directory,sim)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_override_catalog(&mut self,encoded:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_override_catalog(encoded,directory,sim)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn commit_row_visibility(&mut self,effect:(i16,i16,i16))->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.commit_visibility_write(effect)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_visibility(&mut self,value:sims_sim::lot_presenter::visibility::State)->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_visibility(value)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_raster_family(&mut self,input:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_raster_family(input)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_auxiliary(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_auxiliary(encoded,directory)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_slot_records(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_slot_records(encoded,directory)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_slots(&mut self,directory:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_slots(directory)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_sprite_frames(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_sprite_frames(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_object_sprites(&mut self,source:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_object_sprites(source,directory,sim)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_ordinary_objects(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_ordinary_objects(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_ordinary_physical(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_ordinary_physical(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_ordinary(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_ordinary(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_floor(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_floor(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_split(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_split(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_contents(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_contents(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_terrain(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_terrain(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_tail(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row absent")?.bind_tail(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn return_row(&mut self)->Result<()>{self.validate()?;let p=self.paint.as_mut().context("paint absent")?;let r=p.row.as_mut().context("row absent")?;ensure!(r.complete()&&!r.acknowledged(),"row return absent");p.tiles.as_mut().context("tiles absent")?.return_row(r.request())?;r.acknowledge_return()?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_renderer(&mut self,source:&[u8])->Result<()>{let context=self.context.as_ref().context("shared context absent")?;self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row prefix absent")?.bind_renderer(source,context)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn bind_row_placement(&mut self,source:&[u8])->Result<()>{self.paint.as_mut().and_then(|p|p.row.as_mut()).context("row prefix absent")?.bind_placement(source)?;self.validate()}
    pub(in crate::original_session::draw_parent) fn compare_tiles(&self,row:&serde_json::Value)->Result<()>{
        let p=self.paint.as_ref().context("paint absent")?;ensure!(p.at_tile_cut(row),"tile caller differs");
        p.tiles.as_ref().context("tiles absent")?.compare(row,p.surfaces.as_ref().context("surfaces absent")?,p.row.as_ref().zip(self.context.as_ref()))
    }
    pub(in crate::original_session::draw_parent) fn bind_background(&mut self,source:&[u8])->Result<()>{
        ensure!(self.at_entry(),"background binding already executing");
        let p=self.paint.as_mut().context("paint absent")?;ensure!(p.background.is_none(),"background already bound");
        p.background=Some(background::State::import(source)?);self.validate()
    }
    pub(in crate::original_session::draw_parent) fn compare_background(&self,row:&serde_json::Value,scene:&scene::State)->Result<()>{
        let p=self.paint.as_ref().context("paint absent")?;ensure!(p.at_background_cut(row),"background caller differs");
        p.background.as_ref().context("background absent")?.compare(row,&scene.regions)?;
        if row["kind"]=="world_drawing_setup"{
            p.surfaces.as_ref().context("surfaces absent")?.compare(row)?;
            let expected:Vec<u8>=self.context.as_ref().context("context absent")?.state().words().iter().flat_map(|v|v.to_le_bytes()).collect();
            ensure!(crate::original_session::completed_animations::bytes(row["context"]["raw"].as_str().context("background native context absent")?)?==expected,"background shared context differs");
        }
        Ok(())
    }
    pub(in crate::original_session::draw_parent) fn bind_surface_end(&mut self,source:&[u8],native:&[u8])->Result<()>{
        self.validate()?;self.paint.as_mut().and_then(|p|p.surfaces.as_mut()).context("surface end owner absent")?.bind_end(source,native)?;self.validate()
    }
    pub(in crate::original_session::draw_parent) fn compare_surface_end(&self,row:&serde_json::Value)->Result<()>{
        self.validate()?;let p=self.paint.as_ref().context("paint absent")?;ensure!(p.at_surface_end_cut(row),"surface end caller differs");
        p.surfaces.as_ref().context("surface end owner absent")?.compare_end(row)
    }
    pub(in crate::original_session::draw_parent) fn bind_surfaces(
        &mut self,
        source: &[u8],
        native: &[u8],
    ) -> Result<()> {
        ensure!(self.at_entry(), "surface binding already executing");
        let p = self.paint.as_mut().context("paint source absent")?;
        ensure!(p.surfaces.is_none(), "surface owners already bound");
        p.surfaces = Some(surfaces::State::import(source, native)?);
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn compare_surfaces(
        &self,
        row: &serde_json::Value,
    ) -> Result<()> {
        let p = self.paint.as_ref().context("paint source absent")?;
        ensure!(
            p.at_surface_cut(row),
            "surface caller stage differs at{:x}",
            row["pc"].as_u64().unwrap_or(0)
        );
        p.surfaces
            .as_ref()
            .context("surface source absent")?
            .compare(row)
    }
    pub(in crate::original_session::draw_parent) fn compare_paint(
        &self,
        row: &serde_json::Value,
        scene: &scene::State,
        profiler: &Buffer,
    ) -> Result<()> {
        let words = crate::original_session::completed_animations::bytes(
            row["context"]["raw"]
                .as_str()
                .context("paint context bytes absent")?,
        )?;
        let actual = self
            .context
            .as_ref()
            .context("paint context absent")?
            .state()
            .words();
        ensure!(
            actual.iter().flat_map(|v| v.to_le_bytes()).eq(words),
            "paint shared context differs"
        );
        let drawing = self
            .drawing
            .as_ref()
            .context("paint drawing owner absent")?;
        let (enabled, weight) = drawing.region_policy();
        for (key, expected) in [
            ("region_enable", enabled.to_le_bytes().to_vec()),
            ("region_weight", weight.to_bits().to_le_bytes().to_vec()),
            ("camera_mode", vec![drawing.camera_mode()]),
        ] {
            ensure!(
                crate::original_session::completed_animations::bytes(
                    row[key].as_str().context("paint process input absent")?
                )? == expected,
                "paint process input differs: {key}"
            );
        }
        self.paint.as_ref().context("paint source absent")?.compare(
            row,
            scene,
            profiler,
            self.scratch,
        )
    }
}
