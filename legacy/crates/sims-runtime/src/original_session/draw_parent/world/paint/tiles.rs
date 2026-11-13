//! Frozen B inputs and retained first window tile call. The global packed
//! clip and shared surfaces stay with their actual owner. Pixel rows are
//! explicit pending children, not acknowledged by a rendering shortcut.
use super::*;
mod geometry;
mod post_level;
mod region_loop;
use sims_sim::lot_presenter::{
    room_masks::Camera,
    tile_scan::{self, Frame, Inputs},
};
const SOURCE: &str = "188bb8981c998d52a7bc40e2f67900b8e125f5915c239a49e0c171724a338b1b";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Region,
    Clip,
    Scan,
    AfterBase,
    LevelFlag,
    LevelGeometry,
    AfterLevelGeometry,
    AfterLevels,
    Disabled,
    NoRegions,
    ParentLoop,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    stage: Stage,
    flags: Vec<u8>,
    size: i32,
    bounds: [i32; 4],
    camera: Camera,
    diagonal: Vec<u8>,
    diagonal_pointer: u32,
    lot: u32,
    step: [i8; 3],
    fast: u8,
    level: i32,
    index: usize,
    frame: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level_geometry: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level_loop: Option<LevelLoop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    geometry_body: Option<geometry::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level_scan: Option<post_level::State>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    region_source:Option<region_loop::Source>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    region_cursor:Option<region_loop::Cursor>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LevelLoop {
    level: i32,
    rectangle: [i32; 4],
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("tile scan bytes absent")?,
    )
}
impl State {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "tile scan source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "tile scan source epoch differs"
        );
        let m = Memory::new(&v)?;
        let s = &v["scan"];
        let view = m.read(0x09e88960, 0x520)?;
        let lot = word(&m.read(0x6e552c, 4)?, 0);
        let lr = m.read(lot, 0x88)?;
        ensure!(
            s["lot"] == lot && decode(&s["lot_raw"])? == lr,
            "tile scan lot aliases differ"
        );
        let projection = m.read(0x6f00d0, 16)?;
        ensure!(
            decode(&s["projection"])? == projection,
            "tile scan projection differs"
        );
        let zoom = word(&projection, 8);
        ensure!(zoom <= 3, "tile scan zoom unsupported");
        let flags = m.read(0x6c724a, 12)?;
        let step = m.read(0x6e487e, 3)?;
        let pointer = word(&view, 0x1ec);
        let diagonal = m.read(pointer, 128)?;
        ensure!(
            s["diagonal"] == pointer
                && decode(&s["diagonal_raw"])? == diagonal
                && decode(&s["flags"])? == flags
                && decode(&s["step"])? == step,
            "tile scan retained inputs differ"
        );
        let state = Self {
            source: SOURCE.into(),
            stage: Stage::Entry,
            flags,
            size: word(&lr, 0x10) as i32,
            bounds: std::array::from_fn(|i| word(&lr, 0x70 + 4 * i) as i32),
            camera: Camera {
                origin: [word(&projection, 0) as i32, word(&projection, 4) as i32],
                zoom: zoom as u8,
                tile_width: word(&view, 0x58) as i32,
                half_width: word(&view, 0x5c) as i32,
                bottom: word(&view, 0x60) as i32,
                floor_heights: (0..4).map(|i| word(&view, 0x130 - 4 * i) as i32).collect(),
            },
            diagonal,
            diagonal_pointer: pointer,
            lot,
            step: std::array::from_fn(|i| step[i] as i8),
            fast: view[0x17b],
            level: word(&view, 0x18) as i32,
            index: 0,
            frame: None,
            level_geometry: None,
            level_loop: None,
            geometry_body: None,
            level_scan: None,
            region_source:None,
            region_cursor:None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn bind_region_loop(&mut self,encoded:&[u8])->Result<()>{ensure!(self.region_source.is_none(),"region input already bound");self.region_source=Some(region_loop::Source::import(encoded)?);self.validate()}
    pub fn validate(&self) -> Result<()> {
        if let Some(s)=&self.region_source{s.validate()?;}
        if let Some(c)=&self.region_cursor{c.validate()?;}
        ensure!((self.stage==Stage::ParentLoop)==self.region_cursor.is_some(),"parent loop cursor lifetime differs");
        ensure!(
            self.source == SOURCE
                && self.flags.len() == 12
                && self.diagonal.len() == 128
                && (0..=64).contains(&self.size)
                && self.camera.zoom <= 3
                && self.camera.floor_heights.len() == 4,
            "tile scan owner differs"
        );
        ensure!(
            self.frame.is_some() == matches!(self.stage, Stage::Scan | Stage::AfterBase | Stage::LevelFlag | Stage::LevelGeometry | Stage::AfterLevelGeometry | Stage::AfterLevels | Stage::ParentLoop),
            "tile scan caller/child lifetime differs"
        );
        if let Some(f) = &self.frame {
            f.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(self.level_loop.is_some() == matches!(self.stage,Stage::LevelFlag|Stage::LevelGeometry|Stage::AfterLevelGeometry|Stage::AfterLevels|Stage::ParentLoop), "window level loop lifetime differs");
        if let Some(l) = &self.level_loop { ensure!(l.level >= 1 && self.frame.as_ref().is_some_and(Frame::complete), "window level loop lost completed base"); }
        ensure!(self.stage != Stage::LevelGeometry || self.level_geometry.is_some_and(|p| p != 0), "window level geometry owner absent");
        if let Some(scan)=&self.level_scan{scan.validate()?;ensure!(self.stage==Stage::AfterLevelGeometry,"post-level scanner lost caller");}
        if let Some(g)=&self.geometry_body {g.validate()?;ensure!(matches!(self.stage,Stage::LevelGeometry|Stage::AfterLevelGeometry),"geometry iterator lost its caller");}
        Ok(())
    }
    /// Bind the actual model pointer from B, before entering the level loop.
    /// Older saved scans retain their original serialized representation.
    pub fn bind_level_geometry(&mut self, encoded: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(self.level_geometry.is_none() && self.level_loop.is_none(), "window level owner already bound or running");
        ensure!(sims_fixtures::digest(encoded) == SOURCE, "window level source differs");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;
        ensure!(v["tick"] == 89073 && v["executes_target_code"] == false, "window level source epoch differs");
        let m=Memory::new(&v)?;
        ensure!(word(&m.read(0x6e552c,4)?,0)==self.lot,"window level lot alias differs");
        self.level_geometry=Some(word(&m.read(self.lot,0x88)?,0x54));
        self.validate()
    }
    pub fn geometry_request(&self) -> Option<super::Request> {
        if self.stage != Stage::LevelGeometry { return None; }
        let l=self.level_loop.as_ref()?;
        if let Some(g)=&self.geometry_body {
            if let Some(r)=g.pending() {return Some(super::Request::LevelGeometryChild {function:r.kind.entry(),model:r.model,rectangle:l.rectangle,level:l.level});}
            if !g.at_entry() {return None;}
        }
        Some(super::Request::LevelGeometry { model:self.level_geometry?, rectangle:l.rectangle, level:l.level })
    }
    pub(super) fn geometry_input_request(&self) -> Option<super::GeometryInput> {
        if self.stage != Stage::LevelGeometry { return None; }
        self.geometry_body.as_ref().map_or(Some(super::GeometryInput::Iterator), geometry::State::input_request)
    }
    pub fn bind_geometry_body(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.stage==Stage::LevelGeometry && self.geometry_body.is_none(),"geometry iterator binding escaped its entry");
        self.geometry_body=Some(geometry::State::import(source,self.level_geometry.context("geometry model absent")?,self.level_loop.as_ref().context("geometry level absent")?.level)?);self.validate()
    }
    pub fn bind_geometry_child(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.stage==Stage::LevelGeometry,"geometry child lost window level");self.geometry_body.as_mut().context("geometry iterator absent")?.bind_child(source)?;self.validate()
    }
    pub fn bind_geometry_corner(&mut self,source:&[u8],setup:&[u8],graph:&sims_sim::lot_presenter::walls::construction::Graph)->Result<()>{self.validate()?;self.geometry_body.as_mut().context("geometry iterator absent")?.bind_corner(source,setup,graph)?;self.validate()}
    pub fn bind_geometry_segments(&mut self,source:&[u8],graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>)->Result<()> {self.validate()?;let clip=self.level_loop.as_ref().context("geometry segment level absent")?.rectangle;self.geometry_body.as_mut().context("geometry iterator absent")?.bind_segments(source,clip,graph)?;self.validate()}
    pub fn bind_geometry_visible(&mut self,source:&[u8])->Result<()>{self.validate()?;self.geometry_body.as_mut().context("geometry iterator absent")?.bind_visible(source)?;self.validate()}
    pub fn bind_geometry_materials(&mut self,source:&[u8])->Result<()>{self.validate()?;self.geometry_body.as_mut().context("geometry iterator absent")?.bind_materials(source)?;self.validate()}
    pub fn try_bind_geometry_last_side(&mut self)->Result<bool>{let bound=self.geometry_body.as_mut().context("geometry iterator absent")?.try_bind_last_side()?;self.validate()?;Ok(bound)}
    pub fn bind_geometry_side(&mut self)->Result<()>{self.geometry_body.as_mut().context("geometry iterator absent")?.bind_side()?;self.validate()}
    pub fn advance_geometry_child(&mut self,sim:&sims_sim::Simulation,context:&mut sims_vm::scheduler::clock::draw::context::Frame,row:Option<&mut super::row::State>,controls:Option<&sims_vm::interface::state::State>,graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>)->Result<bool> {
        self.validate()?;ensure!(self.stage==Stage::LevelGeometry,"geometry child lost window level");let clip=self.level_loop.as_ref().context("geometry child level absent")?.rectangle;let p=self.geometry_body.as_mut().context("geometry iterator absent")?.advance_child(sim,&self.camera,context,row,controls,graph,clip)?;self.validate()?;Ok(p)
    }
    pub fn row_second(&self)->Option<bool>{self.level_scan.as_ref().and_then(post_level::State::second).or_else(||self.frame.as_ref().map(Frame::second))}
    pub fn row(&self) -> Option<&tile_scan::Request> {
        self.level_scan.as_ref().and_then(post_level::State::row).or_else(||self.frame.as_ref().and_then(Frame::pending))
    }
    pub fn return_row(&mut self,request:&tile_scan::Request)->Result<()>{self.validate()?;if let Some(scan)=&mut self.level_scan{scan.return_row(request)?;}else{self.frame.as_mut().context("tile scan frame absent")?.acknowledge(request,request.point,request.step).map_err(anyhow::Error::msg)?;}self.validate()}
    pub fn advance(
        &mut self,
        regions: &sims_vm::scheduler::clock::draw::regions::List,
        surfaces: &mut surfaces::State,
        flags:u32,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Entry => {
                self.stage = if self.flags[7] == 0 {
                    Stage::Disabled
                } else {
                    Stage::Region
                }
            }
            Stage::Region => {
                if self.index >= regions.len() {
                    self.stage = Stage::NoRegions;
                } else {
                    surfaces.set_window_clip(
                        regions.rectangle(self.index).map_err(anyhow::Error::msg)?,
                    );
                    self.stage = Stage::Clip;
                }
            }
            Stage::Clip => {
                self.frame = Some(Frame::new(1, 1, 1, self.step));
                self.stage = Stage::Scan;
            }
            Stage::Scan => {
                let f = self.frame.as_mut().context("tile scan frame absent")?;
                if !f
                    .advance(Inputs {
                        size: self.size,
                        bounds: self.bounds,
                        camera: &self.camera,
                        diagonal: &self.diagonal,
                        clip: surfaces.window_clip(),
                        fast: self.fast,
                        flags: [self.flags[0], self.flags[1], self.flags[6]],
                    })
                    .map_err(anyhow::Error::msg)?
                {
                    return Ok(false);
                }
                if f.complete() {
                    self.stage = Stage::AfterBase;
                }
            }
            Stage::AfterBase => {
                self.level_loop=Some(LevelLoop { level:1,rectangle:regions.rectangle(self.index).map_err(anyhow::Error::msg)? });
                self.stage=if 1 <= self.level { Stage::LevelFlag } else { Stage::AfterLevels };
            }
            Stage::LevelFlag => {
                if self.flags[2] != 0 {
                    ensure!(self.level_geometry.is_some_and(|p|p!=0),"window level geometry needs its retained native owner");
                    self.stage=Stage::LevelGeometry;
                } else { self.stage=Stage::AfterLevelGeometry; }
            }
            Stage::LevelGeometry => {
                let Some(g)=&mut self.geometry_body else {return Ok(false)};
                if !g.advance()? {return Ok(false);}
                if g.complete() {self.stage=Stage::AfterLevelGeometry;}
            }
            Stage::AfterLevelGeometry => {
                if let Some(scan)=&mut self.level_scan{
                    let inputs=Inputs{size:self.size,bounds:self.bounds,camera:&self.camera,diagonal:&self.diagonal,clip:surfaces.window_clip(),fast:self.fast,flags:[self.flags[0],self.flags[1],self.flags[6]]};
                    if scan.complete(){
                        if self.region_source.is_some(){
                            let l=self.level_loop.as_ref().context("post-level loop absent")?;
                            self.region_cursor=Some(region_loop::Cursor::new(l.level,self.level,self.index,regions)?);
                            self.level_scan=None;self.geometry_body=None;self.stage=Stage::ParentLoop;
                            self.validate()?;return Ok(true);
                        }
                        let l=self.level_loop.as_mut().context("post-level loop absent")?;l.level=l.level.wrapping_add(1);
                        self.level_scan=None;self.geometry_body=None;
                        self.stage=if l.level<=self.level{Stage::LevelFlag}else{Stage::AfterLevels};
                    }else if !scan.advance(inputs,surfaces)?{return Ok(false);}
                }else{
                    let l=self.level_loop.as_ref().context("post-level loop absent")?;
                    let region=word(&regions.header,0).checked_add(u32::try_from(self.index)?.checked_mul(16).context("post-level region offset overflow")?).context("post-level region pointer overflow")?;
                    self.level_scan=Some(post_level::State::new(l.rectangle,region,l.level,self.level,self.step));
                }
            }
            Stage::ParentLoop=>{
                let c=self.region_cursor.as_mut().context("parent cursor absent")?;
                if c.pc()==0x4ad29c{
                    self.index=c.index();self.frame=Some(Frame::new(1,1,1,self.step));self.level_loop=None;self.geometry_body=None;self.level_scan=None;self.region_cursor=None;self.stage=Stage::Scan;
                }else if let Some(level)=c.next_level(){
                    self.level_loop.as_mut().context("level loop absent")?.level=level;self.region_cursor=None;self.stage=Stage::LevelFlag;
                }else if !c.advance(self.region_source.as_ref().context("region source absent")?,regions,surfaces,flags)?{return Ok(false);}
            }
            Stage::AfterLevels | Stage::Disabled | Stage::NoRegions => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        if let Some(c)=&self.region_cursor{return c.pc()==pc;}
        if pc == 0x4ad19e {
            return self.stage == Stage::Entry;
        }
        if pc == 0x4ad2a1 { return self.stage == Stage::AfterBase; }
        if pc == 0x4ad2af { return self.stage == Stage::LevelFlag; }
        if pc == 0x54045b { return self.stage == Stage::LevelGeometry && self.geometry_body.as_ref().is_none_or(|g|g.at_entry()); }
        if self.stage==Stage::LevelGeometry && self.geometry_body.as_ref().is_some_and(|g|g.child_at_cut(pc)){return true;}
        if matches!(pc,0x53db67|0x53c227) {return self.stage==Stage::LevelGeometry && self.geometry_body.as_ref().is_some_and(|g|g.child_at_entry() && g.pending().is_some_and(|r|r.kind.entry()==pc));}
        if let Some(scan)=&self.level_scan {return scan.at_native_cut(pc);}
        if pc == 0x4ad2d9 { return self.stage == Stage::AfterLevelGeometry; }
        matches!(self.stage, Stage::Scan | Stage::AfterBase)
            && self.frame.as_ref().is_some_and(|f| f.at_native_cut(pc))
    }
    pub fn complete(&self)->bool{self.region_cursor.as_ref().is_some_and(|c|c.pc()==0x4a92a2)}
    pub fn compare(&self, row: &serde_json::Value, surfaces: &surfaces::State,graphics:GeometryView<'_>) -> Result<()> {
        self.validate()?;
        let pc = row["pc"].as_u64().context("native tile scan PC absent")? as u32;
        ensure!(
            self.at_native_cut(pc) && row["tick"] == 89081,
            "tile scan boundary differs at{pc:x}"
        );
        if let Some(c)=&self.region_cursor{return c.compare(row,surfaces);}
        if let Some(scan)=&self.level_scan{return scan.compare(row,surfaces);}
        if row["kind"] == "world_drawing_setup" {
            if self.geometry_body.as_ref().is_some_and(|g|g.child_at_cut(pc)) && !matches!(pc,0x53db67|0x53c227) {
                return self.geometry_body.as_ref().unwrap().compare_child(row,graphics);
            }
            if matches!(pc,0x4ad2a1|0x4ad2af|0x54045b|0x4ad2d9|0x53db67|0x53c227) {
                let p=&row["post_base"];
                let raw=decode(&p["parent_frame"]["raw"])?;
                let bp=p["parent_bp"].as_u64().context("post-base parent BP absent")? as u32;
                ensure!(p["parent_frame"]["address"]==bp-0x70 && raw.len()==0xa0,"post-base frame extent differs");
                ensure!(decode(&p["flags"])?==self.flags && decode(&p["clip"])?==surfaces.window_clip().iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>(),"post-base flags/clip differ");
                ensure!(word(&raw,0x48) as i32==self.level,"post-base cached level differs");
                if let Some(l)=&self.level_loop {
                    ensure!(word(&raw,0x44) as i32==l.level,"post-base current level differs");
                    if pc==0x4ad2af { ensure!(row["registers"]["eax"]==l.level,"post-base loop register differs"); }
                    if pc==0x54045b {
                        if let Some(g)=&self.geometry_body {g.compare_inputs(row)?;}
                        let call=&p["wall_call"];let rectangle=decode(&call["rectangle"])?;
                        let stack=decode(&p["stack"]["raw"])?;
                        ensure!(rectangle.len()==16 && std::array::from_fn::<_,4,_>(|i|word(&rectangle,4*i)as i32)==l.rectangle,"post-base rectangle differs");
                        ensure!(row["registers"]["ecx"]==self.level_geometry.context("post-base model owner absent")? && call["return_pc"]==0x4ad2d9u32 && call["level"]==l.level && call["context_pointer"]==row["context"]["pointer"] && call["rectangle_pointer"]==word(&raw,0x58),"post-base callback owners differ");
                        ensure!(word(&stack,0)==0x4ad2d9 && call["rectangle_pointer"]==word(&stack,4) && word(&stack,8)as i32==l.level && call["context_pointer"]==word(&stack,12),"post-base callback ABI differs");
                    }
                    if matches!(pc,0x53db67|0x53c227) {ensure!(row["registers"]["ebx"]==l.level,"geometry child level register differs");self.geometry_body.as_ref().context("geometry iterator absent")?.compare_call(row,l.rectangle)?;}
                }
                return Ok(());
            }
            let s = &row["tile_state"];
            let g = decode(&s["globals"])?;
            let clip: Vec<u8> = surfaces
                .window_clip()
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            ensure!(g[4..12] == clip, "native packed tile clip differs");
            if pc == 0x4ad4bd {
                surfaces.compare(row)?;
                let stack = decode(&s["stack"]["raw"])?;
                ensure!(
                    word(&stack, 0) == 0x4ad2a1
                        && word(&stack, 4) == 0x6f0c10
                        && word(&stack, 8) == 1
                        && word(&stack, 12) == 1
                        && word(&stack, 16) == 1
                        && row["registers"]["ecx"] == 0x09e88960u32,
                    "native first tile call ABI differs"
                );
            }
            return Ok(());
        }
        ensure!(
            row["kind"] == "window_tile_scan",
            "native tile scan kind differs"
        );
        let world = decode(&row["view"])?;
        let lr = decode(&row["lot_raw"])?;
        let projection = decode(&row["projection"])?;
        ensure!(
            row["lot"] == self.lot
                && word(&lr, 0x10) as i32 == self.size
                && std::array::from_fn::<_, 4, _>(|i| word(&lr, 0x70 + 4 * i) as i32)
                    == self.bounds,
            "tile scan lot inputs differ"
        );
        ensure!(
            row["diagonal"] == self.diagonal_pointer
                && decode(&row["diagonal_raw"])? == self.diagonal
                && decode(&row["flags"])? == self.flags
                && decode(&row["fast"])? == [self.fast],
            "tile scan live inputs differ"
        );
        ensure!(
            [word(&projection, 0) as i32, word(&projection, 4) as i32] == self.camera.origin
                && word(&projection, 8) == u32::from(self.camera.zoom)
                && (0..4).map(|i| word(&world, 0x130 - 4 * i) as i32).eq(self
                    .camera
                    .floor_heights
                    .iter()
                    .copied())
                && word(&world, 0x60) as i32 == self.camera.bottom
                && word(&world, 0x18) as i32 == self.level,
            "tile scan projection/level inputs differ"
        );
        ensure!(
            decode(&row["clip"])?
                == surfaces
                    .window_clip()
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            "tile scan shared clip differs"
        );
        let raw = decode(&row["frame"]["raw"])?;
        ensure!(raw.len() == 96, "native tile scan frame extent differs");
        let f = self.frame.as_ref().context("tile scan frame absent")?;
        ensure!(
            raw[0x40 - 0x20..0x40 - 0x1d]
                .iter()
                .copied()
                .eq(f.step().map(|v| v as u8)),
            "tile scan step differs"
        );
        if pc != 0x4ad4df {
            ensure!(
                word(&raw, 0x40 - 0x28) as i32 == f.index()
                    && raw[0x40 - 0x18..0x40 - 0x15]
                        .iter()
                        .copied()
                        .eq(f.point().map(|v| v as u8)),
                "tile scan diagonal/point differs"
            );
            if pc != 0x4ad548 && pc != 0x4ad702 {
                ensure!(
                    [
                        word(&raw, 0x40 - 0x1c) as i32,
                        word(&raw, 0x40 - 0x24) as i32
                    ] == f.screen(),
                    "tile scan projected screen differs"
                );
            }
        }
        if let Some(req) = f.pending() {
            let s = decode(&row["stack"]["raw"])?;
            let bp = row["scan_bp"].as_u64().context("scan bp absent")? as u32;
            ensure!(
                word(&s, 4) == bp - 0x18
                    && word(&s, 8) as i32 == req.count
                    && word(&s, 12) == bp - 0x20
                    && [word(&s, 16) as i32, word(&s, 20) as i32] == req.screen
                    && row["registers"]["ecx"] == 0x09e88960u32,
                "native pending row ABI differs"
            );
        }
        Ok(())
    }
}
