//! Retained4aee4c row prefix. Queries use the computed shared room maps;
//! unsupported split/geometry children stay pending and never finish the row.
use super::*;
use sims_sim::{
    lot_environment::lighting::point::{map_sample, Sample},
    lot_presenter::tile_scan,
};
use sims_vm::scheduler::clock::draw::context::{Frame, LightUpdate};
mod floor;
mod geometry_raster;
mod geometry_catalog;
pub(super) use geometry_catalog::{Material as GeometryMaterial,is_source as is_catalog_source};
mod geometry_side;
mod geometry_side_alt;
pub(super) use geometry_side::{Job as GeometrySideJob,Params as GeometrySideParams};
pub(super) use geometry_raster::{Job as GeometryRasterJob,Params as GeometryRasterParams};
mod placement;
pub mod renderer;
mod split;
mod tail;
mod ordinary;
mod ordinary_objects;
mod object_draw;
mod shadow;
use sims_vm::scheduler::clock::draw::context::placement::PlacementUpdate;
const ROW_SOURCE: &str = "43ae260233863b81d6079a27327acbd3f0ee78bc429b53291a6765bbdfed50c9";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Rotated,
    Light,
    Lower,
    Published,
    Terrain,
    Ground,
    Placing,
    Placed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OrdinaryLight {
    primary: i16,
    value: u32,
    returned: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default,skip_serializing_if="Option::is_none")]
    object_catalog:Option<object_draw::Catalog>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    object_raster:Option<object_draw::raster::Source>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    object_shadow:Option<shadow::Source>,
    source: String,
    request: tile_scan::Request,
    rotation: u8,
    stage: Stage,
    physical: Option<[i32; 2]>,
    rotated: Option<[i8; 3]>,
    light: Option<LightUpdate>,
    split: Option<[i8; 3]>,
    terrain: Option<[u8; 4]>,
    #[serde(default)]
    placement_source: Option<placement::Source>,
    #[serde(default)]
    placement: Option<PlacementUpdate>,
    #[serde(default)]
    renderer: Option<renderer::State>,
    #[serde(default)]
    tail_source: Option<tail::Source>,
    #[serde(default)]
    tail: Option<sims_sim::lot_presenter::row_tail::Frame>,
    #[serde(default)]
    current: Option<[i8; 3]>,
    #[serde(default)]
    current_screen: Option<[i32; 2]>,
    #[serde(default)]
    iteration: i32,
    #[serde(default)]
    acknowledged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    floor_source: Option<floor::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    floor_cursor: Option<floor::Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    split_source: Option<split::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    split_cursor: Option<split::Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ordinary_light: Option<OrdinaryLight>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    geometry_raster:Option<geometry_raster::Source>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    geometry_clip_write:Option<(usize,[i16;2])>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    geometry_side:Option<geometry_side::Source>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    geometry_side_alt:Option<geometry_side_alt::Source>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    geometry_catalog:Option<geometry_catalog::Catalog>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    ordinary:Option<ordinary::Cursor>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    SplitRoom { point: [i8; 3] },
    Terrain { point: [i8; 3] },
    GroundCover { point: [i8; 3] },
    RasterGround,
    GroundRenderer(renderer::Request),
    AfterGroundRenderer,
    RowTail(sims_sim::lot_presenter::row_tail::Request),
    OtherFloorContents,
    Ordinary(ordinary::Pending),
}
impl State {
    pub fn import(source: &[u8], request: tile_scan::Request) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(source) == ROW_SOURCE,
            "row input profile differs"
        );
        let v: serde_json::Value = serde_json::from_slice(source)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "row source epoch differs"
        );
        let m = Memory::new(&v)?;
        let lot = word(&m.read(0x6e552c, 4)?, 0);
        let lr = m.read(lot, 0x88)?;
        let view = m.read(0x09e88960, 0x520)?;
        ensure!(
            request.count > 0 && view[0x17b] != 0 && view[0x50] == 0,
            "row requires its positive-count visible fast-path owner"
        );
        ensure!(
            m.read(0x667664, 4)? == 0x3b808081u32.to_le_bytes()
                && sims_fixtures::digest(&m.read(0x6c741c, 8)?) == "b09540ff36f486fafd91acb451c6d92cc0c31ab1e9f4379a9342bc0cc88df1a6",
            "row light constants differ"
        );
        let s = Self {
            object_catalog:None,
            object_raster:None,
            object_shadow:None,
            source: ROW_SOURCE.into(),
            ordinary:(request.kind==tile_scan::Kind::Ordinary).then(||ordinary::Cursor::new(request.clone())),
            request,
            rotation: word(&lr, 0x84) as u8,
            stage: Stage::Entry,
            physical: None,
            rotated: None,
            light: None,
            split: None,
            terrain: None,
            placement_source: None,
            placement: None,
            renderer: None,
            tail_source: None,
            tail: None,
            current: None,
            current_screen: None,
            iteration: 0,
            acknowledged: false,
            floor_source: None,
            floor_cursor: None,
            split_source: None,
            split_cursor: None,
            ordinary_light: None,
            geometry_raster:None,
            geometry_clip_write:None,
            geometry_side:None,geometry_side_alt:None,geometry_catalog:None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(c)=&self.object_catalog{c.validate()?;}
        if let Some(r)=&self.object_raster{r.validate()?;}
        if let Some(s)=&self.object_shadow{s.validate()?;}
        if let Some(c)=&self.geometry_catalog{c.validate()?;}
        if let Some(s)=&self.geometry_side{s.validate()?;}if let Some(s)=&self.geometry_side_alt{s.validate()?;}
        if let Some(s)=&self.geometry_raster{s.validate()?;}
        ensure!(self.geometry_clip_write.as_ref().is_none_or(|(half,_)|*half<2),"geometry clip publication invalid");
        ensure!(
            self.ordinary_light.is_none()
                || (self.stage == Stage::Rotated
                    && self.split.is_none()
                    && self.split_source.is_some()),
            "ordinary light lookup lost its caller"
        );
        if let Some(c) = &self.split_cursor {
            c.validate()?;
            ensure!(
                c.complete() || self.split.is_some(),
                "split child lost its caller"
            );
        }
        if let Some(s) = &self.split_source {
            s.validate()?;
        }
        if let Some(s) = &self.floor_source {
            s.validate()?;
        }
        if let Some(f) = &self.floor_cursor {
            f.validate()?;
            ensure!(
                self.floor_source.is_some()
                    && self.tail.as_ref().and_then(|t| t.pending())
                        == Some(sims_sim::lot_presenter::row_tail::Request::FloorRaster),
                "floor lost its row continuation"
            );
        }
        if let Some(c)=&self.ordinary{
            c.validate_request(&self.request)?;
            ensure!(self.source==ROW_SOURCE&&self.rotation<=3&&self.stage==Stage::Entry&&self.physical.is_none()&&self.rotated.is_none()&&self.light.is_none()&&self.tail.is_none()&&self.floor_cursor.is_none(),"ordinary row reused fast locals");
            if let Some(p)=&self.placement_source{p.validate()?;}if let Some(r)=&self.renderer{r.validate()?;ensure!(r.complete(),"ordinary row inherited unfinished ground renderer");}
            ensure!(!self.acknowledged||c.complete(),"ordinary row acknowledged before return");return Ok(());
        }
        ensure!(
            self.request.point[..2]
                .iter()
                .all(|&v| (0..64).contains(&v)),
            "row point outside retained rotation table"
        );
        ensure!(
            self.source == ROW_SOURCE
                && self.rotation <= 3
                && self.request.kind == tile_scan::Kind::Fast,
            "row owner/kind differs"
        );
        ensure!(
            self.physical.is_some() == (self.stage != Stage::Entry)
                && self.rotated.is_some() == self.physical.is_some(),
            "row coordinate lifetime differs"
        );
        ensure!(
            self.light.is_some()
                == matches!(
                    self.stage,
                    Stage::Light
                        | Stage::Lower
                        | Stage::Published
                        | Stage::Terrain
                        | Stage::Ground
                        | Stage::Placing
                        | Stage::Placed
                ),
            "row light child lifetime differs"
        );
        ensure!(
            self.split.is_none() || self.stage == Stage::Rotated,
            "row split continuation escaped query"
        );
        if let Some(l) = &self.light {
            l.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                l.complete()
                    == matches!(
                        self.stage,
                        Stage::Published
                            | Stage::Terrain
                            | Stage::Ground
                            | Stage::Placing
                            | Stage::Placed
                    ),
                "row light return differs"
            );
        }
        ensure!(
            self.terrain.is_some()
                == matches!(
                    self.stage,
                    Stage::Terrain | Stage::Ground | Stage::Placing | Stage::Placed
                ),
            "row terrain lifetime differs"
        );
        if let Some(p) = &self.placement_source {
            p.validate()?;
        }
        if let Some(r) = &self.renderer {
            r.validate()?;
            ensure!(
                self.stage == Stage::Placed || r.complete(),
                "ground renderer escaped row placement"
            );
        }
        if let Some(p) = &self.tail_source {
            p.validate()?;
        }
        if let Some(t) = &self.tail {
            t.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.tail_source.is_some() && matches!(self.stage, Stage::Ground | Stage::Placed),
                "row tail escaped contents"
            );
        }
        ensure!(
            !self.acknowledged || self.complete(),
            "row returned before its actual tail"
        );
        ensure!(
            self.placement.is_some() == matches!(self.stage, Stage::Placing | Stage::Placed),
            "row placement lifetime differs"
        );
        ensure!(
            !matches!(self.stage, Stage::Ground | Stage::Placing | Stage::Placed)
                || self.placement_source.is_some(),
            "row placement source absent"
        );
        Ok(())
    }
    fn point(&self) -> [i8; 3] {
        self.current.unwrap_or(self.request.point)
    }
    fn screen(&self) -> [i32; 2] {
        self.current_screen.unwrap_or(self.request.screen)
    }
    pub fn complete(&self) -> bool {
        if let Some(c)=&self.ordinary{return c.complete();}
        self.tail.as_ref().is_some_and(|t| t.complete())
    }
    pub fn diagonal_suspended(&self) -> bool {
        self.floor_cursor
            .as_ref()
            .is_some_and(|f| f.diagonal_pending())
    }
    pub fn acknowledged(&self) -> bool {
        self.acknowledged
    }
    pub fn acknowledge_return(&mut self) -> Result<()> {
        ensure!(
            self.complete() && !self.acknowledged,
            "row return is not pending"
        );
        self.acknowledged = true;
        self.validate()
    }
    fn reset_locals(&mut self, point: [i8; 3], screen: [i32; 2], iteration: i32) {
        self.current = Some(point);
        self.current_screen = Some(screen);
        self.iteration = iteration;
        self.stage = Stage::Entry;
        self.physical = None;
        self.rotated = None;
        self.light = None;
        self.split = None;
        self.split_cursor = None;
        self.ordinary_light = None;
        self.terrain = None;
        self.placement = None;
        self.tail = None;
        self.floor_cursor = None;
    }
    pub fn restart(&mut self, source: &[u8], request: tile_scan::Request) -> Result<()> {
        ensure!(
            sims_fixtures::digest(source) == ROW_SOURCE,
            "row restart source differs"
        );
        self.validate()?;
        ensure!(self.acknowledged, "previous row has not returned");
        let point = request.point;
        let screen = request.screen;
        self.request = request;
        self.acknowledged = false;
        self.reset_locals(point, screen, 0);
        self.ordinary=(self.request.kind==tile_scan::Kind::Ordinary).then(||ordinary::Cursor::new(self.request.clone()));
        self.validate()
    }
    pub fn bind_object_shadow(&mut self,encoded:&[u8])->Result<()>{ensure!(self.object_shadow.is_none(),"shadow source already bound");self.object_shadow=Some(shadow::Source::import(encoded)?);if let Some(c)=&mut self.ordinary{c.resume_shadow();}self.validate()}
    pub fn bind_object_raster(&mut self,encoded:&[u8])->Result<()>{ensure!(self.object_raster.is_none(),"SPR2 raster already bound");self.object_raster=Some(object_draw::raster::Source::import(encoded)?);self.validate()}
    pub(super) fn bind_object_raster_batch(&mut self,encoded:&[u8],targets:sims_vm::scheduler::clock::draw::targets::Buffers)->Result<()>{self.object_raster.as_mut().context("SPR2 raster absent")?.bind_batch(encoded,targets)?;self.validate()}
    pub(super) fn bind_object_raster_mirror(&mut self,encoded:&[u8])->Result<()>{self.object_raster.as_mut().context("SPR2 raster absent")?.bind_mirror(encoded)?;self.validate()}
    pub fn compare_object_raster_batch(&self,q:&serde_json::Value)->Result<()>{self.object_raster.as_ref().context("SPR2 raster absent")?.compare_batch(q)}
    pub fn compare_object_raster(&self,q:&serde_json::Value)->Result<()>{self.ordinary.as_ref().context("ordinary absent")?.compare_raster(self.object_raster.as_ref().context("SPR2 source absent")?,q,self.renderer.as_ref().and_then(|r|r.raster_data()).context("SPR2 planes absent")?)}
    /// Computed shadow stores, not an original CPU-instruction observation.
    pub fn object_shadow_store_progress(&self)->Option<(usize,usize)>{self.ordinary.as_ref()?.shadow_store_progress()}
    pub fn object_raster_store_progress(&self)->Option<(usize,usize)>{self.ordinary.as_ref()?.raster_store_progress()}
    pub fn ordinary_expanded_copy_progress(&self)->Option<usize>{self.ordinary.as_ref()?.expanded_wall_copy_progress()}
    pub fn object_catalog_bound(&self)->bool{self.object_catalog.is_some()}
    pub fn bind_raster_family(&mut self,input:&[u8])->Result<()>{self.object_raster.as_mut().context("object raster absent")?.bind_family(input)?;self.validate()}
    pub fn bind_object_catalog(&mut self,encoded:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{ensure!(self.ordinary.is_some()&&self.object_catalog.is_none(),"ordinary catalog already bound or not active");self.object_catalog=Some(object_draw::Catalog::import(encoded,directory,sim)?);self.validate()}
    pub fn bind_override_catalog(&mut self,encoded:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.object_catalog.as_mut().context("object catalog absent")?.bind_override_catalog(encoded,directory,sim)?;self.validate()}
    pub fn visibility_write(&self)->Option<(i16,i16,i16)>{self.ordinary.as_ref()?.visibility_write()}
    pub fn bind_visibility(&mut self,value:sims_sim::lot_presenter::visibility::State)->Result<()>{ensure!(self.ordinary.as_ref().is_some_and(|c|c.at_native_cut(0x4ad8ac)),"visibility binding requires ordinary row entry");self.object_catalog.as_mut().context("object catalog absent")?.bind_visibility(value)?;self.validate()}
    pub fn commit_visibility_write(&mut self,effect:(i16,i16,i16))->Result<()>{self.ordinary.as_mut().context("ordinary row absent")?.commit_visibility_write(effect)?;self.validate()}
    pub(super) fn bind_object_auxiliary(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{self.object_catalog.as_mut().context("drawing catalog absent")?.bind_auxiliary(encoded,directory)?;self.validate()}
    pub(super) fn bind_object_slot_records(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{self.object_catalog.as_mut().context("drawing catalog absent")?.bind_slot_records(encoded,directory)?;self.validate()}
    pub(super) fn bind_object_slots(&mut self,directory:&[u8])->Result<()>{self.object_catalog.as_mut().context("drawing catalog absent")?.bind_slots(directory)?;self.validate()}
    pub fn bind_object_sprite_frames(&mut self,source:&[u8])->Result<()>{self.ordinary.as_mut().context("ordinary row absent")?.bind_frames(source)?;self.validate()}
    pub fn bind_object_sprites(&mut self,source:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.ordinary.as_mut().context("ordinary row absent")?.bind_sprites(source,directory,sim)?;self.validate()}
    pub(super) fn set_second(&mut self,second:bool)->Result<()>{if let Some(o)=&mut self.ordinary{o.set_second(second)?;}Ok(())}
    pub(super) fn ordinary_second(&self)->Option<bool>{self.ordinary.as_ref().map(ordinary::Cursor::second)}
    pub fn bind_ordinary_objects(&mut self,source:&[u8])->Result<()>{self.ordinary.as_mut().context("ordinary row is not active")?.bind_objects(source)?;self.validate()}
    pub fn bind_ordinary_physical(&mut self,source:&[u8])->Result<()>{self.ordinary.as_mut().context("ordinary row is not active")?.bind_physical(source)?;self.validate()}
    pub fn bind_ordinary(&mut self,source:&[u8])->Result<()>{self.ordinary.as_mut().context("ordinary row is not active")?.bind(source)?;self.validate()}
    pub fn bind_terrain(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        self.renderer
            .as_mut()
            .context("terrain renderer absent")?
            .bind_terrain(source)?;
        self.validate()
    }
    pub fn bind_contents(&mut self, source: &[u8]) -> Result<()> {
        self.tail_source
            .as_mut()
            .context("tail inputs absent")?
            .bind_contents(source)?;
        self.validate()
    }
    pub fn bind_floor(&mut self, source: &[u8]) -> Result<()> {
        ensure!(self.floor_source.is_none(), "floor inputs already bound");
        self.floor_source = Some(floor::Source::import(source)?);
        self.validate()
    }
    pub fn bind_floor_sprite(&mut self, source: &[u8]) -> Result<()> {
        self.floor_source
            .as_mut()
            .context("floor source absent")?
            .bind_sprite(source)?;
        self.validate()
    }
    pub fn bind_split(&mut self, source: &[u8]) -> Result<()> {
        ensure!(self.split_source.is_none(), "split inputs already bound");
        self.split_source = Some(split::Source::import(source)?);
        self.validate()
    }
    pub fn split_inputs(&self, sim: &sims_sim::Simulation) -> Result<serde_json::Value> {
        let inputs = self
            .split_source
            .as_ref()
            .context("split source absent")?
            .inputs(
                sim,
                self.physical.context("split physical point absent")?,
                self.rotation,
            )?;
        Ok(serde_json::to_value(inputs)?)
    }
    pub fn bind_raster(&mut self, source: &[u8], color: &[u8], depth: &[u8]) -> Result<()> {
        self.renderer
            .as_mut()
            .context("raster owner absent")?
            .bind_raster(source, color, depth)?;
        self.validate()
    }
    pub fn extend_depth_rows(&mut self, source: &[u8]) -> Result<()> {
        self.placement_source
            .as_mut()
            .context("placement source absent")?
            .extend_rows(source)?;
        self.validate()
    }
    pub fn bind_tail(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(self.tail_source.is_none(), "tail input already bound");
        self.tail_source = Some(tail::Source::import(source)?);
        self.validate()
    }
    pub fn request(&self) -> &tile_scan::Request {
        &self.request
    }
    pub fn bind_placement(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(
            self.placement_source.is_none(),
            "placement source already bound"
        );
        self.placement_source = Some(placement::Source::import(source)?);
        self.validate()
    }
    pub(super) fn advance_geometry_placement(&mut self,update:&mut PlacementUpdate,simulation:&sims_sim::Simulation,context:&mut Frame)->Result<bool>{
        self.placement_source.as_mut().context("geometry shared placement source absent")?.advance(update,simulation,context)
    }
    pub(super) fn geometry_scene(&self)->Result<u32>{Ok(self.floor_source.as_ref().context("geometry Scene identity absent")?.native_scene())}
    pub(super) fn geometry_descriptor(&self,point:[i8;3],rotation:u8)->Result<[u8;4]>{self.floor_source.as_ref().context("geometry floor descriptor owner absent")?.geometry_descriptor(point,rotation)}
    pub(super) fn bind_geometry_catalog(&mut self,meta:&[u8],raw:&[u8])->Result<()>{
        let c=geometry_catalog::Catalog::import(meta,raw)?;if let Some(old)=&self.geometry_catalog{c.extends(old)?;}self.geometry_raster.as_mut().context("catalog requires retained raster owners")?.extend_catalog(&c)?;if let Some(side)=&mut self.geometry_side{side.extend_catalog(&c)?;}self.geometry_catalog=Some(c);self.validate()
    }
    pub(super) fn lookup_portal(&self,sim:&sims_sim::Simulation,point:[u8;3],rotation:u8,mask:u32)->Result<Option<(u32,i16)>>{self.geometry_catalog.as_ref().context("portal catalog absent")?.portal(sim,point,rotation,mask)}
    pub(super) fn lookup_geometry_material(&self,key:u32)->Result<Option<GeometryMaterial>>{self.geometry_catalog.as_ref().map(|c|c.material(key)).transpose()}
    pub(super) fn bind_geometry_diagonal_columns(&mut self,source:&[u8])->Result<()>{self.geometry_raster.as_mut().context("geometry raster absent")?.bind_diagonal_columns(source)?;self.validate()}
    pub(super) fn bind_geometry_raster(&mut self,source:&[u8],material:&[u8],targets:sims_vm::scheduler::clock::draw::targets::Buffers)->Result<()>{
        if let Some(s)=self.geometry_raster.as_mut(){ensure!(s.targets==targets,"geometry target identity changed");s.extend(source,material)?;}
        else{self.geometry_raster=Some(geometry_raster::Source::import(source,material,targets)?);}self.validate()
    }
    pub(super) fn geometry_raster_ready(&self)->bool{self.geometry_raster.is_some()}
    pub(super) fn bind_geometry_side(&mut self,source:&[u8],material:&[u8])->Result<()>{ensure!(self.geometry_side.is_none(),"side source already bound");let targets=self.geometry_raster.as_ref().context("side current targets absent")?.targets;self.geometry_side=Some(geometry_side::Source::import(source,material,targets)?);self.validate()}
    pub(super) fn advance_geometry_side(&mut self,job:&mut GeometrySideJob,sim:&sims_sim::Simulation,context:&mut Frame,camera:&sims_sim::lot_presenter::room_masks::Camera)->Result<bool>{job.advance(self.geometry_side.as_mut().context("side B source absent")?,self.renderer.as_mut().and_then(|r|r.raster_data_mut()).context("side planes absent")?,sim,context,camera,self.placement_source.as_mut().context("side placement owner absent")?,self.geometry_side_alt.as_mut())}
    pub(super) fn compare_geometry_side(&self,job:&GeometrySideJob,q:&serde_json::Value,context:&Frame)->Result<()>{job.compare(q,self.geometry_side.as_ref().context("side source absent")?,self.geometry_side_alt.as_ref(),context)}
    pub(super) fn geometry_side_cache(&self)->Result<u32>{Ok(self.geometry_side.as_ref().context("side B cache absent")?.cache())}
    pub(super) fn geometry_corner_vertical_offset(&self,zoom:u8)->Result<i32>{self.geometry_side.as_ref().context("corner B depth table absent")?.corner_vertical_offset(zoom)}
    pub(super) fn bind_geometry_side_alt(&mut self,input:&[u8],color:&[u8],mask:&[u8])->Result<()>{ensure!(self.geometry_side_alt.is_none(),"alternate side source already bound");let targets=self.geometry_raster.as_ref().context("alternate side targets absent")?.targets;self.geometry_side_alt=Some(geometry_side_alt::Source::import(input,color,mask,targets)?);self.validate()}
    pub(super) fn advance_geometry_raster(&mut self,job:&mut GeometryRasterJob,context:&Frame)->Result<bool>{
        job.advance(self.geometry_raster.as_mut().context("geometry resources absent")?,self.renderer.as_mut().and_then(|r|r.raster_data_mut()).context("geometry shared planes absent")?,context)
    }
    pub(super) fn compare_geometry_raster(&self,job:&GeometryRasterJob,q:&serde_json::Value,context:&Frame)->Result<()>{job.compare(q,self.geometry_raster.as_ref().context("geometry resources absent")?,context)}
    pub(super) fn request_geometry_clip(&mut self,clip:[i32;4],half:usize)->Result<()>{
        ensure!(self.geometry_clip_write.is_none()&&half<2,"geometry clip publication already pending");let packed=[clip[1]as i16,clip[0]as i16,clip[3]as i16,clip[2]as i16];self.geometry_clip_write=Some((half,[packed[half*2],packed[half*2+1]]));Ok(())
    }
    pub(super) fn take_geometry_clip(&mut self)->Option<(usize,[i16;2])>{self.geometry_clip_write.take()}
    pub fn bind_renderer(&mut self, source: &[u8], context: &Frame) -> Result<()> {
        self.validate()?;
        ensure!(
            self.renderer.is_none() && self.stage == Stage::Placed,
            "ground renderer binding escaped placement return"
        );
        self.renderer = Some(renderer::State::import(
            source,
            self.point(),
            context.state().words()[8],
        )?);
        self.validate()
    }
    /// Compare surviving shared owners without reading a returned child's stack.
    pub(super) fn compare_shared(&self, row: &serde_json::Value, context: &Frame) -> Result<()> {
        self.validate()?;
        if let Some(s)=&self.geometry_raster{s.compare(row)?;}if let Some(s)=&self.geometry_side_alt{s.compare(row)?;}
        context.validate().map_err(anyhow::Error::msg)?;
        let raw = row.get("shared_context").unwrap_or(&row["context"]["raw"]);
        let native = crate::original_session::completed_animations::bytes(
            raw.as_str().context("shared graphic context absent")?,
        )?;
        ensure!(context.state().words().iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>() == native,
            "shared graphic context differs");
        if let Some(observed)=row["visible_geometry"].get("placement_caches") {
            let items=observed.as_array().context("visible placement caches absent")?;
            ensure!(items.len()==2,"visible placement cache count differs");
            let c=&self.placement_source.as_ref().context("visible shared cache owner absent")?.caches;
            for (item,(address,cache)) in items.iter().zip([(0x703f28u32,&c.alpha),(0x703f38u32,&c.beta)]) {
                let raw=crate::original_session::completed_animations::bytes(item["raw"].as_str().context("visible cache bytes absent")?)?;
                ensure!(item["pointer"]==address&&raw.len()==8&&word(&raw,0)==cache.flags&&word(&raw,4)==cache.value,"visible shared placement cache differs");
            }
        }
        if let Some(buffers) = row.get("buffers") {
            ensure!(buffers.as_array().is_some_and(|a| a.len() == 2), "shared graphic plane count differs");
        }
        self.renderer.as_ref().context("shared graphic renderer absent")?.compare_owned_buffers(row)
    }
    pub fn compare(&self, row: &serde_json::Value, context: &Frame) -> Result<()> {
        self.validate()?;
        if let Some(c)=&self.ordinary{if let Some(cat)=&self.object_catalog{cat.compare_slot_call(row)?;c.compare_auxiliary(row,cat)?;}c.compare(row,context)?;if c.shadow_active(){c.compare_shadow(self.object_shadow.as_ref().context("shadow source absent")?,row,context,self.renderer.as_ref().and_then(|r|r.raster_data()).context("shadow planes absent")?)?;}return Ok(());}
        context.validate().map_err(anyhow::Error::msg)?;
        let pc = row["pc"].as_u64().context("native row PC absent")? as u32;
        ensure!(
            row["kind"] == "window_tile_scan" && row["tick"] == 89081 && self.at_native_cut(pc),
            "native row cursor differs"
        );
        let native = crate::original_session::completed_animations::bytes(
            row["shared_context"]
                .as_str()
                .context("native row context absent")?,
        )?;
        ensure!(
            context
                .state()
                .words()
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
                == native,
            "native row context differs at{pc:x}"
        );
        if self.stage != Stage::Entry && !matches!(pc, 0x4ad680 | 0x4ad88f) {
            let raw = crate::original_session::completed_animations::bytes(
                row["row_frame"]["raw"]
                    .as_str()
                    .context("native row locals absent")?,
            )?;
            ensure!(
                raw[0x210..0x213] == self.rotated.unwrap().map(|v| v as u8),
                "native row rotated point differs"
            );
            if self.stage != Stage::Rotated {
                ensure!(
                    [word(&raw, 0xd8) as i32, word(&raw, 0xdc) as i32] == self.physical.unwrap(),
                    "native row fine center differs"
                );
            }
        }
        if pc == 0x4af029 {
            let raw = crate::original_session::completed_animations::bytes(
                row["row_frame"]["raw"]
                    .as_str()
                    .context("native terrain locals absent")?,
            )?;
            ensure!(
                raw[0x1e8..0x1ec] == self.terrain.unwrap(),
                "native row terrain corners differ"
            );
        }
        if pc == 0x4aef90 {
            let raw = crate::original_session::completed_animations::bytes(
                row["stack"]["raw"]
                    .as_str()
                    .context("native light call stack absent")?,
            )?;
            ensure!(
                word(&raw, 0) == self.light.as_ref().unwrap().input_bits(),
                "native unclamped light input differs"
            );
        }
        if pc == 0x467482 {
            let raw = crate::original_session::completed_animations::bytes(
                row["split_light"]["input"]["physical"]
                    .as_str()
                    .context("native light point absent")?,
            )?;
            ensure!(
                [word(&raw, 0) as i32, word(&raw, 4) as i32] == self.physical.unwrap()
                    && row["split_light"]["input"]["level"] == 1,
                "native split light input differs"
            );
        }
        if pc == 0x4674e4 {
            if let Some(light) = &self.ordinary_light {
                ensure!(
                    crate::original_session::completed_animations::bytes(
                        row["split_light"]["primary_room"]["raw"]
                            .as_str()
                            .context("ordinary native room absent")?
                    )? == light.primary.to_le_bytes(),
                    "ordinary primary room differs"
                );
            }
        }
        if let Some(p) = &self.placement {
            p.validate_words(context.state().words())
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(c) = &self.split_cursor {
            if c.at_native_cut(pc) {
                c.compare(row)?;
            }
        }
        if let Some(p) = &self.placement_source {
            p.compare(row)?;
        }
        if let Some(r) = &self.renderer {
            r.compare(row)?;
        }
        if let Some(f) = &self.floor_cursor {
            if f.at_native_cut(pc) {
                f.compare(
                    self.floor_source.as_ref().context("floor inputs absent")?,
                    row,
                    context.state().words(),
                    self.renderer
                        .as_ref()
                        .and_then(|r| r.raster_data())
                        .context("floor raster planes absent")?,
                )?;
            }
        }
        if matches!(
            pc,
            0x4b0111 | 0x4b011a | 0x4b012c | 0x4b012f | 0x4b0135 | 0x4b0138 | 0x4b0147 | 0x4b0155
        ) {
            let t = self.tail.as_ref().context("native tail frame absent")?;
            let raw = crate::original_session::completed_animations::bytes(
                row["row_frame"]["raw"]
                    .as_str()
                    .context("native tail locals absent")?,
            )?;
            ensure!(
                raw[0x214..0x217] == t.point().map(|v| v as u8)
                    && word(&raw, 0x174) as i32 == t.index()
                    && [word(&raw, 0x238) as i32, word(&raw, 0x23c) as i32] == t.screen(),
                "original row local loop result differs"
            );
        }
        Ok(())
    }
    pub fn pending(&self) -> Option<Request> {
        if let Some(c)=&self.ordinary{return c.pending().map(Request::Ordinary);}
        if let Some(t) = &self.tail {
            return t.pending().map(Request::RowTail);
        }

        if let Some(point) = self.split {
            return Some(Request::SplitRoom { point });
        }
        match self.stage {
            Stage::Published => Some(Request::Terrain {
                point: self.point(),
            }),
            Stage::Terrain => Some(Request::GroundCover {
                point: self.rotated.unwrap(),
            }),
            Stage::Ground => Some(Request::OtherFloorContents),
            Stage::Placed => self
                .renderer
                .as_ref()
                .map_or(Some(Request::RasterGround), |r| {
                    if r.complete() {
                        Some(Request::AfterGroundRenderer)
                    } else {
                        r.pending().map(Request::GroundRenderer)
                    }
                }),
            _ => None,
        }
    }
    pub fn advance(
        &mut self,
        simulation: &sims_sim::Simulation,
        context: &mut Frame,
        clip: [i16; 4],
        classify: Option<&mut classification::State>,
    ) -> Result<bool> {
        self.validate()?;
        if let Some(c)=&mut self.ordinary{
            let progress=c.advance(simulation,self.rotation,self.split_source.as_ref(),context,self.placement_source.as_mut(),self.floor_source.as_ref(),classify,clip,self.object_catalog.as_ref(),self.object_raster.as_mut(),self.object_shadow.as_ref(),self.renderer.as_mut().and_then(|r|r.raster_data_mut()))?;
            self.validate()?;return Ok(progress);
        }
        if let Some(t) = &mut self.tail {
            if t.pending() == Some(sims_sim::lot_presenter::row_tail::Request::FloorRaster) {
                if self
                    .floor_cursor
                    .as_ref()
                    .is_some_and(|f| f.draw_complete())
                {
                    t.finish_floor().map_err(anyhow::Error::msg)?;
                    self.floor_cursor = None;
                    return Ok(true);
                }
                let Some(source) = &self.floor_source else {
                    return Ok(false);
                };
                if self.floor_cursor.is_none() {
                    let lot = simulation
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .context("floor live lot absent")?;
                    let p = self.rotated.context("floor physical point absent")?;
                    ensure!(
                        p[0] >= 0
                            && p[1] >= 0
                            && (p[0] as usize) < lot.size
                            && (p[1] as usize) < lot.size,
                        "floor physical point outside grid"
                    );
                    let floor = lot.floor(i32::from(p[2])).context("floor level absent")?;
                    let i = p[0] as usize * lot.size + p[1] as usize;
                    self.floor_cursor = Some(floor::Cursor::new(
                        t.point(),
                        t.screen(),
                        self.terrain.context("floor height corners absent")?,
                        self.rotation,
                        context.state().words()[0x20 / 4],
                        floor.source.coverings[i],
                        floor.source.walls[i].segments,
                    ));
                }
                let f = self.floor_cursor.as_mut().unwrap();
                if f.diagonal_pending() {
                    return f.advance_diagonal(
                        source,
                        self.split_source
                            .as_ref()
                            .context("diagonal room directory absent")?,
                        self.placement_source
                            .as_ref()
                            .context("diagonal placement absent")?,
                        simulation,
                        self.renderer
                            .as_mut()
                            .and_then(|r| r.raster_data_mut())
                            .context("diagonal planes absent")?,
                        context,
                        clip,
                    );
                }
                if f.draw_bound(source) {
                    return f.advance_draw(
                        source,
                        self.placement_source
                            .as_ref()
                            .context("sprite depth source absent")?,
                        simulation,
                        self.renderer
                            .as_mut()
                            .and_then(|r| r.raster_data_mut())
                            .context("sprite raster planes absent")?,
                        context.state().words(),
                        clip,
                    );
                }
                return f.advance(source, context.state().words(), simulation);
            }
            if t.pending() == Some(sims_sim::lot_presenter::row_tail::Request::NextTile) {
                let (p, s, i) = (t.point(), t.screen(), t.index());
                self.reset_locals(p, s, i);
                self.validate()?;
                return Ok(true);
            }
            return self
                .tail_source
                .as_ref()
                .context("tail source absent")?
                .advance(
                    t,
                    simulation,
                    self.rotation,
                    self.rotated.context("row rotated point absent")?,
                );
        }
        if let Some(l) = &self.light {
            l.validate_words(context.state().words())
                .map_err(anyhow::Error::msg)?;
        }
        match self.stage {
            Stage::Entry => {
                let [x, y, z] = self.point();
                let r = match 4u8.wrapping_sub(self.rotation) & 3 {
                    1 => [63 - y, x, z],
                    2 => [63 - x, 63 - y, z],
                    3 => [y, 63 - x, z],
                    _ => [x, y, z],
                };
                self.rotated = Some(r);
                self.physical = Some([(i32::from(y) << 4) | 8, (i32::from(x) << 4) | 8]);
                self.stage = Stage::Rotated;
            }
            Stage::Rotated => {
                if let Some(light) = &mut self.ordinary_light {
                    if light.returned {
                        self.light = Some(LightUpdate::new(light.value));
                        self.ordinary_light = None;
                        self.stage = Stage::Light;
                    } else {
                        light.returned = true;
                    }
                    self.validate()?;
                    return Ok(true);
                }
                if self.split.is_some() {
                    let Some(source) = &self.split_source else {
                        return Ok(false);
                    };
                    let c = self.split_cursor.as_mut().context("split cursor absent")?;
                    c.advance(source, simulation, self.physical.unwrap(), self.rotation)?;
                    if c.complete() {
                        self.light =
                            Some(LightUpdate::new(c.result().context("split result absent")?));
                        self.split = None;
                        self.stage = Stage::Light;
                    }
                    self.validate()?;
                    return Ok(true);
                }
                let lot = simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("row lot absent")?;
                let env = &simulation
                    .room_environment
                    .as_ref()
                    .context("row room maps absent")?
                    .manager;
                let floor = lot.floor(1).context("row ground floor absent")?;
                match map_sample(
                    self.physical.unwrap(),
                    1,
                    self.rotation,
                    lot.size,
                    &floor.rooms.primary,
                    env.light_bytes.first().context("row light bytes absent")?,
                )
                .map_err(anyhow::Error::msg)?
                {
                    Sample::Value(value) => {
                        if self.split_source.is_some() {
                            let p = self.rotated.unwrap();
                            self.ordinary_light = Some(OrdinaryLight {
                                primary: floor.rooms.primary
                                    [p[0] as usize * lot.size + p[1] as usize],
                                value,
                                returned: false,
                            });
                        } else {
                            self.light = Some(LightUpdate::new(value));
                            self.stage = Stage::Light;
                        }
                    }
                    Sample::Split { point } => {
                        self.split = Some(point);
                        self.split_cursor = Some(split::Cursor::new());
                    }
                }
            }
            Stage::Light | Stage::Lower => {
                ensure!(
                    context
                        .advance_light(self.light.as_mut().unwrap())
                        .map_err(anyhow::Error::msg)?,
                    "row light child stalled"
                );
                self.stage = if self.light.as_ref().unwrap().complete() {
                    Stage::Published
                } else {
                    Stage::Lower
                };
            }
            Stage::Published => {
                let lot = simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("row terrain lot absent")?;
                self.terrain = Some(
                    sims_sim::lot_presenter::room_masks::terrain_corners(
                        lot.size,
                        &lot.heights,
                        self.point(),
                        self.rotation,
                    )
                    .map_err(anyhow::Error::msg)?,
                );
                self.stage = Stage::Terrain;
            }
            Stage::Terrain => {
                let Some(source) = self.placement_source.as_ref() else {
                    return Ok(false);
                };
                let lot = simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("ground selection lot absent")?;
                let p = self.rotated.unwrap();
                let [x, y] = [i32::from(p[0]), i32::from(p[1])];
                let index = |x: i32, y: i32| -> Result<usize> {
                    ensure!(
                        x >= 0 && y >= 0 && (x as usize) < lot.size && (y as usize) < lot.size,
                        "ground lookup outside retained grid"
                    );
                    Ok(x as usize * lot.size + y as usize)
                };
                let floor = lot
                    .floor(i32::from(p[2]))
                    .context("ground selection floor absent")?;
                let draw = sims_sim::lot_presenter::ground::selected(
                    source.ground,
                    p,
                    floor.source.coverings[index(x, y)?],
                    lot.size,
                    source.active,
                    &lot.floor(1)
                        .context("ground covering owner absent")?
                        .source
                        .coverings,
                    &lot.water,
                )
                .map_err(anyhow::Error::msg)?;
                if draw {
                    let [x, y, _] = p;
                    self.placement = Some(PlacementUpdate::new(
                        [(i32::from(y) << 4) | 8, (i32::from(x) << 4) | 8],
                        1,
                        -1,
                        self.rotation,
                    ));
                    self.stage = Stage::Placing;
                } else {
                    self.stage = Stage::Ground;
                }
            }
            Stage::Placing => {
                let update = self.placement.as_mut().context("placement cursor absent")?;
                if !self
                    .placement_source
                    .as_mut()
                    .context("placement source absent")?
                    .advance(update, simulation, context)?
                {
                    return Ok(false);
                }
                if update.complete() {
                    self.stage = Stage::Placed;
                    let point = self.point();
                    if let Some(r) = &mut self.renderer {
                        r.restart(point, context.state().words()[8])?;
                    }
                }
            }
            Stage::Placed => {
                if self.renderer.as_ref().is_some_and(|r| r.complete()) {
                    if self.tail_source.is_none() {
                        return Ok(false);
                    }
                    self.tail = Some(sims_sim::lot_presenter::row_tail::Frame::new(
                        self.point(),
                        self.rotated.unwrap(),
                        self.screen(),
                        self.request.step,
                        self.request.count,
                        self.iteration,
                    ));
                    self.validate()?;
                    return Ok(true);
                }
                let screen = self.screen();
                let corners = self.terrain.context("rectangle corners absent")?;
                let Some(r) = self.renderer.as_mut() else {
                    return Ok(false);
                };
                r.supply_arguments(screen, corners)?;
                let lot = simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("ground wrapper covering absent")?;
                let p = self.rotated.unwrap();
                ensure!(
                    p[0] >= 0
                        && p[1] >= 0
                        && (p[0] as usize) < lot.size
                        && (p[1] as usize) < lot.size,
                    "ground wrapper covering outside grid"
                );
                let covering = lot
                    .floor(1)
                    .context("ground covering floor absent")?
                    .source
                    .coverings[p[0] as usize * lot.size + p[1] as usize];
                if !r.advance(
                    simulation,
                    self.placement_source
                        .as_ref()
                        .context("ground placement inputs absent")?,
                    covering,
                    clip,
                    context.state().words()[22] & 1 != 0,
                )? {
                    return Ok(false);
                }
            }
            Stage::Ground => {
                if self.tail_source.is_none() {
                    return Ok(false);
                }
                self.tail = Some(sims_sim::lot_presenter::row_tail::Frame::new(
                    self.point(),
                    self.rotated.unwrap(),
                    self.screen(),
                    self.request.step,
                    self.request.count,
                    self.iteration,
                ));
            }
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        if let Some(c)=&self.ordinary{return c.at_native_cut(pc);}
        if self
            .floor_cursor
            .as_ref()
            .is_some_and(|f| f.at_native_cut(pc))
        {
            return true;
        }
        if let Some(light) = &self.ordinary_light {
            if pc == 0x4674e4 {
                return !light.returned;
            }
            if pc == 0x46767f {
                return light.returned;
            }
        }
        if self
            .split_cursor
            .as_ref()
            .is_some_and(|c| c.at_native_cut(pc))
        {
            return true;
        }
        if matches!(
            pc,
            0x4af453
                | 0x4aed23
                | 0x4af479
                | 0x4af485
                | 0x467690
                | 0x4af4da
                | 0x4af4e7
                | 0x4afae4
                | 0x4aff8d
                | 0x4c6383
                | 0x4affb1
                | 0x4c811f
                | 0x4c8e0d
                | 0x4c8f52
                | 0x4c8152
                | 0x49f42d
                | 0x49f628
                | 0x49f949
                | 0x4b954a
                | 0x4c82c9
                | 0x4b0000
        ) {
            if let Some(f) = &self.floor_cursor {
                return f.at_native_cut(pc);
            }
        }
        if matches!(pc, 0x4ad680 | 0x4ad88f) {
            return self.acknowledged;
        }
        if let Some(t) = &self.tail {
            return t.at_native_cut(pc);
        }
        if self.stage == Stage::Placed {
            if let Some(r) = &self.renderer {
                return r.at_native_cut(pc);
            }
        }
        match pc {
            0x4aee4c => self.stage == Stage::Entry,
            0x467482 => {
                self.stage == Stage::Rotated
                    && self.split.is_none()
                    && self.ordinary_light.is_none()
            }
            0x4aeefb => self.stage == Stage::Rotated && self.split.is_none(),
            0x4aef89 | 0x4aef90 => self.stage == Stage::Light,
            0x4b4544 => self.stage == Stage::Lower,
            0x4aef95 => self.stage == Stage::Published,
            0x4af029 => self.stage == Stage::Terrain,
            0x4af194 => {
                self.stage == Stage::Placing
                    && self
                        .placement
                        .as_ref()
                        .is_some_and(|p| p.at_native_cut(0x4b9613))
            }
            pc => self.placement.as_ref().is_some_and(|p| p.at_native_cut(pc)),
        }
    }
    /// A shared getter may be entered consecutively with different arguments.
    /// Its address alone cannot identify the suspended continuation.
    pub fn at_native_observation(&self, row: &serde_json::Value) -> Result<bool> {
        let pc = row["pc"].as_u64().context("native row PC absent")? as u32;
        if !self.at_native_cut(pc) {
            return Ok(false);
        }
        if let Some(c)=&self.ordinary{return c.matches_observation(row);}
        if pc == 0x49cd43 {
            return self
                .renderer
                .as_ref()
                .context("terrain renderer absent")?
                .matches_shade_call(row);
        }
        if pc == 0x467690 {
            if let Some(c) = &self.split_cursor {
                if c.at_native_cut(pc) {
                    return c.matches_wall_call(row);
                }
            }
            if let Some(f) = &self.floor_cursor {
                if f.at_native_cut(pc) {
                    return f.matches_wall_call(row);
                }
            }
            return self
                .renderer
                .as_ref()
                .context("terrain renderer absent")?
                .matches_wall_call(row);
        }
        Ok(true)
    }
}
