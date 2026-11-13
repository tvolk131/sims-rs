//!53dcc4..540480, with explicit pending visible/polygon branches. The clipped
//! branch still executes all descriptor, projection and rectangle services.
use super::*;
use sims_sim::lot_presenter::room_masks::Camera;
use sims_vm::scheduler::clock::draw::context::Frame as ContextFrame;
mod visible;
type Side=crate::original_session::draw_parent::world::paint::row::GeometrySideJob;
type Row = crate::original_session::draw_parent::world::paint::row::State;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    ProjectCall,
    ProjectReturned,
    TerrainCall,
    TerrainReturned,
    CurrentCall,
    CurrentReturned,
    PreviousCall,
    PreviousReturned,
    PreviousClassCall,
    PreviousClassReturned,
    CurrentClassCall,
    CurrentClassReturned,
    NextClassCall,
    NextClassReturned,
    ZoomCall,
    ZoomReturned,
    BoundsCall,
    BoundsReturned,
    Dispatch,
    Thunk,
    ApiCall,
    ApiReturned,
    Clip,
    Tail,
    Returned,
    Side,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    model: u32,
    begin: u32,
    records: Vec<[u8; 6]>,
    step: [u8; 2],
    original: [u8; 3],
    unadjusted: [u8; 3],
    point: [u8; 3],
    zoom: u8,
    orientation: u32,
    mask: u32,
    rotation: u8,
    index: usize,
    stage: Stage,
    screen: Option<[i32; 2]>,
    terrain: Option<[u8; 4]>,
    flags: [u8; 4],
    heights: [u8; 2],
    active: u8,
    previous_class: bool,
    current_class: bool,
    next_class: bool,
    result: bool,
    ///53e7d7 writes the endpoint helper result into native local -a1.
    #[serde(default,skip_serializing_if="Option::is_none")]
    endpoint_mask: Option<u8>,
    context_zoom: Option<u32>,
    rectangle: Option<[i32; 4]>,
    delta: Option<[i32; 2]>,
    overlaps: Option<bool>,
    clip: [i32; 4],
    #[serde(default,skip_serializing_if="Option::is_none")]
    visible:Option<visible::State>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    side:Option<Side>,
}
impl State {
    pub(super) fn input_request(&self) -> Option<super::super::super::super::GeometryInput> {
        if let Some(v) = &self.visible { return v.input_request(); }
        (self.stage == Stage::Clip && self.overlaps == Some(true))
            .then_some(super::super::super::super::GeometryInput::Visible)
    }
    pub fn import(
        encoded: &[u8],
        model: u32,
        point: [u8; 3],
        zoom: u8,
        clip: [i32; 4],
        graph: Option<&sims_sim::lot_presenter::walls::construction::Graph>,
    ) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        ensure!(matches!(source.as_str(), MODE_INPUT | VECTOR_INPUT), "geometry segment source differs");
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        let m = Memory::new(&v)?;
        let raw = model_input(&v, &m, &source, model)?;
        ensure!(
            raw[0x48] == 0,
            "geometry segment model/mode differs"
        );
        let view = decode(&v["setup"]["process_child_4_raw"])?;
        ensure!(
            view[0x50] == 0,
            "geometry segment visibility callback is not implemented"
        );
        let lot = word(&m.read(0x6e552c, 4)?, 0);
        let rotation = word(&m.read(lot, 0x88)?, 0x84);
        ensure!(rotation < 4, "geometry segment rotation differs");
        let (begin, end) = (word(&raw, 0x30), word(&raw, 0x34));
        ensure!(
            begin <= end && (end - begin) % 6 == 0 && end - begin <= 6144,
            "geometry segment bounds differ"
        );
        let bytes = segment_input(&v, &m, &source, model, &raw)?;
        let mut records: Vec<[u8; 6]> = bytes
            .chunks_exact(6)
            .map(|v| v.try_into().unwrap())
            .collect();
        let orientation = word(&raw, 0x10);
        let original = raw[4..7].try_into()?;
        if source == VECTOR_INPUT {
            let graph = graph.context("first-vector drawing requires its current scene descriptors")?;
            graph.validate().map_err(anyhow::Error::msg)?;
            ensure!(graph.rotation == rotation as u8, "geometry scene rotation differs");
            let models = v["first_vector"]["models"].as_array().context("first-vector models absent")?;
            let index = models.iter().position(|v| v["pointer"] == model).context("scene model identity absent")?;
            let floor = &graph.state.floors[0];
            ensure!(floor.edges.len() == models.len(), "scene first-vector extent differs");
            let edge = &graph.state.edges[floor.edges[index]];
            ensure!(edge.kind as u32 == orientation
                && graph.geometry.positions[edge.ends[0]].map(|v| v as u8) == original
                && edge.segments.len() == records.len(), "scene model/segment ownership differs");
            for (record, current) in records.iter_mut().zip(&edge.segments) {
                let [a,b] = current.patterns[0].to_le_bytes();
                let [c,d] = current.patterns[1].to_le_bytes();
                ensure!(record[1..] == [current.style,a,b,c,d], "scene immutable descriptor inputs differ");
                // Cutaway owns these flags. Do not restore their stale B values
                // after the current scene has already computed its update.
                *record = [current.flags,current.style,a,b,c,d];
            }
        }
        let s = Self {
            visible:None,
            side:None,
            model,
            begin,
            records,
            step: raw[10..12].try_into()?,
            original,
            unadjusted: original,
            point,
            zoom,
            orientation,
            mask: level_geometry::orientation_mask(orientation),
            rotation: rotation as u8,
            index: 0,
            stage: Stage::Entry,
            screen: None,
            terrain: None,
            flags: [0; 4],
            heights: [0; 2],
            active: 0,
            previous_class: false,
            current_class: false,
            next_class: false,
            result: false,
            endpoint_mask: None,
            context_zoom: None,
            rectangle: None,
            delta: None,
            overlaps: None,
            clip,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::Entry
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(s)=&self.side{s.validate()?;ensure!(matches!(self.stage,Stage::NextClassReturned|Stage::CurrentClassReturned|Stage::Side),"side child lost its caller");}
        if let Some(v)=&self.visible {v.validate()?;ensure!(self.stage==Stage::Clip&&self.overlaps==Some(true),"visible geometry lost its pending clip owner");}
        ensure!(
            self.index <= self.records.len()
                && (self.index == self.records.len()) == self.complete()
                || self.stage == Stage::Entry && self.records.is_empty(),
            "geometry segment cursor lifetime differs"
        );
        ensure!(self.rotation < 4, "geometry segment rotation invalid");
        Ok(())
    }
    pub fn bind_visible(&mut self,light:u32,side:u8)->Result<()> {
        self.validate()?;ensure!(self.stage==Stage::Clip&&self.overlaps==Some(true)&&self.visible.is_none(),"visible geometry binding escaped its actual branch");
        self.visible=Some(visible::State::new(self.point,self.rotation,self.flags[1],light,self.current(),side,self.zoom,self.mask,self.rectangle.unwrap(),self.active));self.validate()
    }
    pub fn bind_materials(&mut self,source:&[u8])->Result<()>{self.validate()?;self.visible.as_mut().context("visible owner absent")?.bind_materials(source)?;self.validate()}
    pub fn try_bind_last_side(&mut self,light:u32)->Result<bool>{
        //53dea6/53dec7 bypass the next descriptor query on the final record,
        //then53deef still enters the ordinary side dispatch for this point.
        if self.stage==Stage::CurrentClassReturned&&self.index+1==self.records.len()&&self.active==1&&self.unadjusted!=self.original&&self.mask==2&&self.orientation==3&&self.previous_class&&!self.result&&self.side.is_none(){self.bind_side(light)?;return Ok(true)}
        Ok(false)
    }
    pub fn bind_side(&mut self,light:u32)->Result<()>{
        self.validate()?;ensure!((self.stage==Stage::NextClassReturned||(self.stage==Stage::CurrentClassReturned&&self.index+1==self.records.len()&&!self.result))&&self.side.is_none()&&self.active==1&&self.mask==2&&self.orientation==3&&self.previous_class&&(self.current_class||self.stage==Stage::CurrentClassReturned&&!self.result),"side branch differs");
        let open=|style:u8|matches!(style,3|5|6|15|23);
        ensure!(!open(self.current()[1])&&!open(self.previous()[1]),"side door-edge services are pending");
        let mut flags=10u8;if self.current()[0]&2!=0{flags&=!8}if self.previous()[0]&4!=0{flags&=!2}
        self.side=Some(Side::new(crate::original_session::draw_parent::world::paint::row::GeometrySideParams{point:self.point,rotation:self.rotation,zoom:self.zoom,height:self.heights[0],flags,alternate:if self.current()[0]&2!=0{0}else{self.active},screen:self.screen.context("side projection absent")?,light,clip:self.clip})?);self.validate()
    }
    fn current(&self) -> [u8; 6] {
        self.records[self.index]
    }
    fn previous(&self) -> [u8; 6] {
        if self.index == 0 {
            [0; 6]
        } else {
            self.records[self.index - 1]
        }
    }
    fn cursor(&self) -> u32 {
        self.begin.wrapping_add(self.index as u32 * 6)
    }
    fn prepare_classes(&mut self) {
        self.stage = Stage::PreviousClassCall;
    }
    fn prepare_zoom(&mut self,light:u32,graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>) -> Result<()> {
        //53def0 avoids side geometry at the model's initial point. Other
        //points enter the mask dispatch before the common top rectangle.
        if self.unadjusted != self.original {
            ensure!(
                matches!(self.mask,1|2) && self.active == 0,
                "geometry side polygon branch is pending"
            );
            let previous = self.previous();
            let current = self.current();
            //53df95 /53e091: axis2 uses bits2/8 and reverses the
            // descriptor edge flags relative to axis1's bits1/4.
            let (near,far,current_flag,previous_flag)=if self.mask==1{(1u8,4u8,4u8,2u8)}else{(2u8,8u8,2u8,4u8)};
            let mut flags = near|far;
            if self.previous_class != self.current_class {
                flags &= if self.current_class { !near } else { !far };
            } else if !self.previous_class {
                flags &= !(near|far);
            }
            if current[0] & current_flag != 0 {
                flags &= !far;
            }
            if previous[0] & previous_flag != 0 {
                flags &= !near;
            }
            if flags!=0 {
                //53e126/e14f: auxiliary edge repair is independent of the
                // ordinary/alternate renderer selection in53e184/e2fa.
                let open=|style:u8|matches!(style,3|5|6|15|23);
                ensure!(self.previous_class==self.current_class&&!(open(previous[1])&&previous[0]&near!=0),"side auxiliary edge repair is pending");
                let side=Side::new(crate::original_session::draw_parent::world::paint::row::GeometrySideParams{point:self.point,rotation:self.rotation,zoom:self.zoom,height:self.heights[0],flags,alternate:0,screen:self.screen.context("side projection absent")?,light,clip:self.clip})?;
                self.side=Some(side);self.stage=Stage::Side;return self.validate()
            }
        }
        if self.mask==32 {
            if self.index+1==self.records.len() {
                //53edb6 reads model+24's current corner, not its captured B
                //bytes. Match the already-bound logical edge, rejecting aliases.
                let graph=graph.context("geometry diagonal top endpoint service is pending")?;
                graph.validate().map_err(anyhow::Error::msg)?;
                ensure!(graph.rotation==self.rotation,"diagonal endpoint rotation differs");
                let mut endpoint=None;
                for edge in &graph.state.edges {
                    if edge.kind as u32==self.orientation
                        && graph.geometry.positions[edge.ends[0]].map(|v|v as u8)==self.original
                        && edge.segments.len()==self.records.len() {
                        ensure!(endpoint.is_none(),"diagonal endpoint owner is ambiguous");
                        ensure!(edge.segments.iter().zip(&self.records).all(|(s,r)|
                            s.flags==r[0]&&s.style==r[1]
                            &&s.patterns==[u16::from_le_bytes([r[2],r[3]]),u16::from_le_bytes([r[4],r[5]])]),
                            "diagonal endpoint descriptors differ from current scene");
                        let corner=&graph.state.corners[edge.ends[1]];
                        endpoint=Some(corner.base&!corner.masks[2]);
                    }
                }
                let exposed=endpoint.context("diagonal endpoint owner absent")?;
                self.endpoint_mask=Some(exposed);
                //53e7f2..fa rechecks EDI against end-6. The read-only helper
                //cannot change this final-descriptor identity. Preserve that
                //last comparison: exposed !=16 alone does NOT skip this record.
                let last=self.begin.wrapping_add(self.records.len() as u32*6).wrapping_sub(6);
                if exposed!=16&&self.current_class&&self.active==0&&self.cursor()!=last {
                    self.stage=Stage::Tail;return Ok(())
                }
            } else if self.active==0&&self.records[self.index+1][0]&1==0
                &&self.current_class&&self.next_class {
                //53e7a9..c8 uses next ACTIVE, independently of next class.
                self.stage=Stage::Tail;return Ok(())
            }
        }
        self.stage = Stage::ZoomCall;
        Ok(())
    }
    pub fn advance(
        &mut self,
        sim: &sims_sim::Simulation,
        camera: &Camera,
        context: &mut ContextFrame,
        row:Option<&mut Row>,
        controls:Option<&sims_vm::interface::state::State>,outside_light:u32,graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>,
    ) -> Result<bool> {
        self.validate()?;
        if self.stage==Stage::Side{let s=self.side.as_mut().context("side child absent")?;let progress=row.context("side row owner absent")?.advance_geometry_side(s,sim,context,camera)?;if s.complete(){self.side=None;self.stage=Stage::ZoomCall;}self.validate()?;return Ok(progress)}
        if let Some(v)=&mut self.visible{let progress=v.advance(sim,context,row,self.clip,controls,outside_light)?;if v.complete(){self.visible=None;self.stage=Stage::Tail;}self.validate()?;return Ok(progress)}
        match self.stage {
            Stage::Entry => {
                self.stage = if self.records.is_empty() {
                    Stage::Returned
                } else {
                    Stage::ProjectCall
                }
            }
            Stage::ProjectCall => {
                ensure!(
                    usize::from(camera.zoom) < camera.floor_heights.len(),
                    "geometry projection height absent"
                );
                self.screen = Some(level_geometry::project(camera, self.point));
                self.stage = Stage::ProjectReturned;
            }
            Stage::ProjectReturned => self.stage = Stage::TerrainCall,
            Stage::TerrainCall => {
                let lot = sim
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("geometry current terrain owner absent")?;
                self.terrain = Some(
                    sims_sim::lot_presenter::room_masks::terrain_corners(
                        lot.size,
                        &lot.heights,
                        self.point.map(|v| v as i8),
                        self.rotation,
                    )
                    .map_err(anyhow::Error::msg)?,
                );
                self.stage = Stage::TerrainReturned;
            }
            Stage::TerrainReturned => {
                self.flags = self.terrain.unwrap();
                self.heights = level_geometry::segment_heights(self.orientation, self.flags);
                self.stage = if self.current()[1] == 255 {
                    Stage::PreviousCall
                } else {
                    Stage::CurrentCall
                };
            }
            Stage::CurrentCall => {
                self.result = level_geometry::special_kind(u32::from(self.current()[1]));
                self.stage = Stage::CurrentReturned;
            }
            Stage::CurrentReturned => {
                if self.result {
                    self.stage = Stage::PreviousCall;
                } else {
                    self.active = self.current()[0] & 1;
                    self.prepare_classes();
                }
            }
            Stage::PreviousCall => {
                if self.previous()[1] == 255 {
                    self.active = 0;
                    self.prepare_classes();
                } else {
                    self.result = level_geometry::special_kind(u32::from(self.previous()[1]));
                    self.stage = Stage::PreviousReturned;
                }
            }
            Stage::PreviousReturned => {
                self.active = if self.result {
                    0
                } else {
                    self.previous()[0] & 1
                };
                self.prepare_classes();
            }
            Stage::PreviousClassCall => {
                self.result = level_geometry::special_kind(u32::from(self.previous()[1]));
                self.stage = Stage::PreviousClassReturned;
            }
            Stage::PreviousClassReturned => {
                self.previous_class = !self.result;
                self.stage = Stage::CurrentClassCall;
            }
            Stage::CurrentClassCall => {
                self.result = level_geometry::special_kind(u32::from(self.current()[1]));
                self.stage = Stage::CurrentClassReturned;
            }
            Stage::CurrentClassReturned => {
                self.current_class = !self.result;
                if self.index + 1 == self.records.len() {
                    self.next_class = true;
                    // The final descriptor bypasses the next-class query.
                    // Its supported side child is selected by computed state.
                    if self.side.is_none() { self.try_bind_last_side(outside_light)?; }
                    if self.side.is_some(){self.stage=Stage::Side}else{self.prepare_zoom(outside_light,graph)?;}
                } else {
                    self.stage = Stage::NextClassCall;
                }
            }
            Stage::NextClassCall => {
                self.result =
                    level_geometry::special_kind(u32::from(self.records[self.index + 1][1]));
                self.stage = Stage::NextClassReturned;
            }
            Stage::NextClassReturned => {
                self.next_class = !self.result;
                //53df95..53e1f7: use the existing validated side constructor
                // without an out-of-band diagnostic driver choosing the call.
                if self.side.is_none() && self.unadjusted!=self.original
                    && self.active==1 && self.mask==2 && self.orientation==3
                    && self.previous_class && self.current_class {
                    self.bind_side(outside_light)?;
                }
                if self.side.is_some(){self.stage=Stage::Side}else{self.prepare_zoom(outside_light,graph)?;}
            }
            Stage::ZoomCall => {
                self.context_zoom = Some(context.state().words()[0x20 / 4]);
                self.stage = Stage::ZoomReturned;
            }
            Stage::ZoomReturned => self.stage = Stage::BoundsCall,
            Stage::BoundsCall => {
                let z = self.context_zoom.unwrap();
                ensure!(z <= 3, "geometry bounds zoom storage is unobserved");
                let height = *camera
                    .floor_heights
                    .get(z as usize)
                    .context("geometry rectangle floor height absent")?;
                self.rectangle = Some(level_geometry::segment_bounds(self.mask, z as u8, height));
                self.stage = Stage::BoundsReturned;
            }
            Stage::BoundsReturned => {
                let [x, y] = self.screen.unwrap();
                let shift = u32::from(self.zoom) + 1;
                let mut delta=[x,y.wrapping_sub(i32::from(self.heights[1]).wrapping_shl(shift))];
                if self.mask==32 {
                    //53dc89/92 retain8<<zoom;53e886..a4 applies the
                    //diagonal one-pixel left shift and this vertical bias.
                    delta[0]=delta[0].wrapping_sub(1);
                    delta[1]=delta[1].wrapping_add(8i32.wrapping_shl(u32::from(self.zoom)))
                        .wrapping_add(4i32.wrapping_shr(3u32.wrapping_sub(u32::from(self.zoom))));
                }
                self.delta = Some(delta);
                self.stage = Stage::Dispatch;
            }
            Stage::Dispatch => self.stage = Stage::Thunk,
            Stage::Thunk => self.stage = Stage::ApiCall,
            Stage::ApiCall => {
                let d = self.delta.unwrap();
                let r = self.rectangle.as_mut().unwrap();
                for i in 0..4 {
                    r[i] = r[i].wrapping_add(d[i % 2]);
                }
                self.stage = Stage::ApiReturned;
            }
            Stage::ApiReturned => {
                self.overlaps = Some(level_geometry::segment_overlaps(
                    self.rectangle.unwrap(),
                    self.clip,
                ));
                self.stage = Stage::Clip;
            }
            Stage::Clip => {
                if self.overlaps == Some(true) {
                    return Ok(false);
                }
                self.stage = Stage::Tail;
            }
            Stage::Tail => {
                for i in 0..2 {
                    self.point[i] = self.point[i].wrapping_add(self.step[i]);
                    self.unadjusted[i] = self.unadjusted[i].wrapping_add(self.step[i]);
                }
                self.index += 1;
                self.stage = if self.index == self.records.len() {
                    Stage::Returned
                } else {
                    Stage::ProjectCall
                };
            }
            Stage::Returned => return Ok(false),
            Stage::Side=>unreachable!(),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_cut(&self, pc: u32) -> bool {
        if self.stage==Stage::Side{return self.side.as_ref().is_some_and(|s|s.at_cut(pc))}
        if let Some(v)=&self.visible{return v.at_cut(pc)}
        if pc==0x53e8f8{return self.stage==Stage::Clip&&self.overlaps==Some(true)}
        matches!(
            (pc, self.stage),
            (0x53dd65, Stage::ProjectCall)
                | (0x53dd6a, Stage::ProjectReturned)
                | (0x53dd8e, Stage::TerrainCall)
                | (0x53dd93, Stage::TerrainReturned)
                | (0x53de18, Stage::CurrentCall)
                | (0x53de1d, Stage::CurrentReturned)
                | (0x53de3a, Stage::PreviousCall)
                | (0x53de3f, Stage::PreviousReturned)
                | (0x53de78, Stage::PreviousClassCall)
                | (0x53de7d, Stage::PreviousClassReturned)
                | (0x53de8a, Stage::CurrentClassCall)
                | (0x53de8f, Stage::CurrentClassReturned)
                | (0x53dead, Stage::NextClassCall)
                | (0x53deb2, Stage::NextClassReturned)
                | (0x53e84d, Stage::ZoomCall)
                | (0x53e852, Stage::ZoomReturned)
                | (0x53e861, Stage::BoundsCall)
                | (0x53e866, Stage::BoundsReturned)
                | (0x53e8bc, Stage::Dispatch)
                | (0x2f12874, Stage::Thunk)
                | (0x688d82c0, Stage::ApiCall)
                | (0x53e8c2, Stage::ApiReturned)
                | (0x53e8f2, Stage::Clip)
                | (0x53ed7b, Stage::Tail)
                | (0x540480, Stage::Returned)
        )
    }
    pub fn compare(&self, q: &serde_json::Value,graphics:GeometryView<'_>) -> Result<()> {
        self.validate()?;
        if self.stage==Stage::Side{let(row,context)=graphics.context("side comparison row absent")?;return row.compare_geometry_side(self.side.as_ref().unwrap(),q,context)}
        if let Some(v)=&self.visible{return v.compare(q,graphics)}
        let pc = q["pc"].as_u64().context("geometry segment PC absent")? as u32;
        ensure!(self.at_cut(pc), "geometry segment stage differs");
        if self.complete() {
            ensure!(
                q["geometry_body_event"]["deallocated_child_locals"] == true,
                "geometry return must not inspect deallocated child locals"
            );
            return Ok(());
        }
        let p = &q["geometry_prefix"];
        let model = decode(&p["model_raw"])?;
        ensure!(
            model[4..7] == self.original
                && model[10..12] == self.step
                && word(&model, 0x10) == self.orientation
                && word(&model, 0x30) == self.begin
                && word(&model, 0x34) == self.begin.wrapping_add(self.records.len() as u32 * 6),
            "geometry consumed model aliases differ"
        );
        let stack = decode(&p["stack"]["raw"])?;
        if self.stage == Stage::ApiCall {
            let d = self.delta.unwrap();
            ensure!(
                word(&stack, 8) as i32 == d[0] && word(&stack, 12) as i32 == d[1],
                "geometry OffsetRect scalar arguments differ"
            );
            return Ok(());
        }
        let f = decode(&p["frame"]["raw"])?;
        let bp = p["bp"].as_u64().context("geometry child BP absent")? as u32;
        if let Some(mask)=self.endpoint_mask {ensure!(f[0x178-0xa1]==mask,"geometry diagonal endpoint helper result differs");}
        ensure!(
            q["registers"]["esi"] == self.model
                && q["registers"]["edi"] == self.cursor()
                && f[0x178 - 0x1c..0x178 - 0x19] == self.point
                && f[0x178 - 0x44..0x178 - 0x41] == self.unadjusted,
            "geometry segment cursor/points differ"
        );
        match self.stage {
            Stage::ProjectCall => ensure!(
                word(&stack, 0) == bp - 0x68
                    && word(&stack, 4) == bp - 0x80
                    && word(&stack, 8) == bp - 0x1c,
                "geometry projection arguments differ"
            ),
            Stage::ProjectReturned | Stage::TerrainCall | Stage::TerrainReturned => (),
            Stage::CurrentCall
            | Stage::PreviousCall
            | Stage::PreviousClassCall
            | Stage::CurrentClassCall
            | Stage::NextClassCall => {
                let value = match self.stage {
                    Stage::CurrentCall | Stage::CurrentClassCall => self.current()[1],
                    Stage::NextClassCall => self.records[self.index + 1][1],
                    _ => self.previous()[1],
                };
                ensure!(
                    word(&stack, 0) == u32::from(value),
                    "geometry classification argument differs"
                );
            }
            Stage::CurrentReturned
            | Stage::PreviousReturned
            | Stage::PreviousClassReturned
            | Stage::CurrentClassReturned
            | Stage::NextClassReturned => ensure!(
                q["registers"]["eax"]
                    .as_u64()
                    .context("geometry classification AL absent")?
                    & 255
                    == u64::from(self.result),
                "geometry classification result differs"
            ),
            Stage::ZoomReturned => ensure!(
                q["registers"]["eax"] == self.context_zoom.unwrap(),
                "geometry context zoom result differs"
            ),
            Stage::BoundsCall => ensure!(
                word(&stack, 0) == bp - 0xc0
                    && word(&stack, 4) == bp - 0x50
                    && word(&stack, 8) == self.context_zoom.unwrap(),
                "geometry bounds arguments differ"
            ),
            Stage::Dispatch | Stage::Thunk => {
                let d = self.delta.unwrap();
                ensure!(
                    word(&stack, 0) == bp - 0xc0
                        && word(&stack, 4) as i32 == d[0]
                        && word(&stack, 8) as i32 == d[1],
                    "geometry protected import arguments differ"
                );
            }
            _ => (),
        }
        if self.stage != Stage::ProjectCall {
            let s = self.screen.unwrap();
            ensure!(
                word(&f, 0x178 - 0x68) as i32 == s[0] && word(&f, 0x178 - 0x80) as i32 == s[1],
                "geometry projected screen differs"
            );
        }
        if self.stage == Stage::TerrainReturned {
            ensure!(
                f[0x178 - 0x110..0x178 - 0x10c] == self.terrain.unwrap(),
                "geometry terrain service result differs"
            );
        }
        if !matches!(
            self.stage,
            Stage::ProjectCall
                | Stage::ProjectReturned
                | Stage::TerrainCall
                | Stage::TerrainReturned
        ) {
            ensure!(
                f[0x178 - 0x2c..0x178 - 0x28] == self.flags
                    && f[0x178 - 0x25] == self.heights[0]
                    && f[0x178 - 0x1d] == self.heights[1],
                "geometry consumed corner/height locals differ"
            );
        }
        if let Some(r) = self.rectangle {
            if matches!(
                self.stage,
                Stage::BoundsReturned
                    | Stage::Dispatch
                    | Stage::Thunk
                    | Stage::ApiReturned
                    | Stage::Clip
                    | Stage::Tail
            ) {
                ensure!(
                    (0..4).all(|i| word(&f, 0x178 - 0xc0 + 4 * i) as i32 == r[i]),
                    "geometry rectangle local differs"
                );
            }
        }
        if self.stage == Stage::Clip {
            let negative = q["registers"]["eflags"]
                .as_u64()
                .context("geometry clip flags absent")?
                & 0x80
                != 0;
            ensure!(
                negative == !self.overlaps.unwrap(),
                "geometry signed clip result differs"
            );
        }
        Ok(())
    }
}
