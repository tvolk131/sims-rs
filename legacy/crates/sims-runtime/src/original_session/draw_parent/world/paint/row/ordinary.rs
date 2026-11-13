//!4ad8ac ordinary row. Shared light/depth owners are borrowed from the retained
//!renderer; unsupported drawing children stay pending before their effects.
use super::*;
const SOURCE: &str = "7f4a5452fbfcd7a89f2682f9a48abd708def1451bfc02a4f6df43043529cb536";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Source {
    identity: String,
    ui: u32,
    view: Vec<u8>,
    flags: Vec<u8>,
    editor: u32,
    terrain_overlay: u32,
    pool: bool,
}
impl Source {
    fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "ordinary row source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "ordinary row source epoch differs"
        );
        let m = Memory::new(&v)?;
        let ui = word(&m.read(word(&m.read(0x6f2a58, 4)?, 0) + 0xb0, 4)?, 0);
        let lr = m.read(word(&m.read(0x6e552c, 4)?, 0), 0x88)?;
        ensure!(
            sims_fixtures::digest(&m.read(0x416920, 4)?) == "916ae10aa3fd76a4d5b25b2fb66e9df0fd04957c7ab10930268d1ed7808be689",
            "ordinary UI getter bytes differ"
        );
        Ok(Self {
            identity: SOURCE.into(),
            ui: word(&m.read(ui, 12)?, 8),
            view: m.read(0x09e88960, 0x520)?,
            flags: m.read(0x6c7248, 14)?,
            editor: word(&m.read(0x6e4ad4, 4)?, 0),
            terrain_overlay: word(&m.read(0x6f06b8, 4)?, 0),
            pool: word(&lr, 0x60) != 0,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pending {
    Source,
    Editor,
    ExpandedWalls,
    SplitLight,
    Objects,
    UpperFloor,
    TerrainOverlay,
    Pool,
    ObjectSource,
    RegistryRepair,
    ObjectDraw,
    ObjectVisibility,
    ContainedSlotDraw,
    AuxiliaryObjectDraw,
    SpriteFrame,
    SpriteRaster,
    ObjectDrawReturn,
    ObjectNext,
    ObjectShadow,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    UiCall,
    UiReturned,
    Copied,
    Loop,
    RotateCall,
    Rotated,
    TerrainCall,
    TerrainReturned,
    HeadCall,
    HeadReturned,
    FineCall,
    FineReturned,
    LightCall,
    LightReturned,
    PublishCall,
    Publishing,
    PublishReturned,
    PlaceFineCall,
    PlaceFineReturned,
    PlaceCall,
    Placing,
    PlaceReturned,
    LookupSelected,
    LookupStored,
    ObjectGate,
    Objects,
    ObjectLoop,
    QuadCall,
    QuadReturned,
    AfterFloor,
    OverlayDone,
    PoolCall,
    PoolReturned,
    PoolDone,
    Tail,
    Epilogue,
    Returned,
    Blocked(Pending),
    Shadow,
    ObjectPlaceCall,
    ObjectPlacing,
    ObjectPlaceReturned,
    ObjectClearCall,
    ObjectClearReturned,
    ObjectUiCall,
    ObjectUiReturned,
    ObjectDrawCall,
    Drawing,
    ObjectDrawReturned,
    ObjectFlagsCall,
    ObjectFlagsReturned,
    ObjectNextCall,
    ObjectNextReturned,
    ExpandedCall,
    ExpandedReturned,
    ExpandedCopy,
    ObjectNextExecuting,
    VisibilityMerged,
    LightEntry,
    Splitting,
    LightMapped,
    LightResult,
    HideRead,HideRemembered,HideSaved,HideWritten,VisibilityRestoreRead,VisibilityRestoreStore,VisibilityRestored,
}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
struct ExpandedQuery {
    result:sims_sim::surface_remap::ExpandedWall,
    copied:usize,
}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
struct VisibilityOverride {id:i16,prior:i16}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Cursor {
    source: Option<Source>,
    request: tile_scan::Request,
    #[serde(default,skip_serializing_if="Option::is_none")]
    second_return:Option<bool>,
    stage: Stage,
    index: i32,
    point: [i8; 3],
    screen: [i32; 2],
    rotated: Option<[i8; 3]>,
    terrain: Option<[u8; 4]>,
    quad: Option<[u8; 4]>,
    head: Option<i16>,
    covering: Option<u16>,
    fine: Option<[i32; 2]>,
    placement_fine: Option<[i32; 2]>,
    light: Option<u32>,
    light_cursor: Option<LightUpdate>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    split_cursor: Option<split::Cursor>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    normal_light_primary: Option<i16>,
    placement: Option<PlacementUpdate>,
    pool_value: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    objects: Option<ordinary_objects::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected: Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    draw_source: Option<object_draw::Source>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    draw_cursor: Option<object_draw::Cursor>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    next_selected:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    shadow_job:Option<shadow::Job>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    expanded:Option<ExpandedQuery>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    selected_id:Option<i16>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    next_id:Option<i16>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    physical_next:Option<sims_sim::lot_presenter::physical_next::Next>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    visibility:Option<VisibilityOverride>,
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("ordinary bytes absent")?,
    )
}
impl Cursor {
    pub fn set_second(&mut self,second:bool)->Result<()>{ensure!(self.stage==Stage::Entry,"ordinary caller rebound after entry");self.second_return=second.then_some(true);Ok(())}
    pub fn second(&self)->bool{self.second_return==Some(true)}
    fn return_pc(&self)->u32{if self.second(){0x4ad885}else{0x4ad6ab}}

    pub fn shadow_store_progress(&self) -> Option<(usize, usize)> {
        self.shadow_job.as_ref()?.store_progress()
    }
    pub fn validate_request(&self, r: &tile_scan::Request) -> Result<()> {
        ensure!(&self.request == r, "ordinary request changed");
        self.validate()
    }
    pub fn new(request: tile_scan::Request) -> Self {
        let point = request.point;
        let screen = request.screen;
        Self {
            source: None,
            request,
            second_return:None,
            stage: Stage::Entry,
            index: 0,
            point,
            screen,
            rotated: None,
            terrain: None,
            quad: None,
            head: None,
            covering: None,
            fine: None,
            placement_fine: None,
            light: None,
            light_cursor: None,
            split_cursor: None,
            normal_light_primary: None,
            placement: None,
            pool_value: None,
            objects: None,
            selected: None,
            draw_source:None,
            draw_cursor:None,
            next_selected:None,
            shadow_job:None,
            expanded:None,
            selected_id:None,
            next_id:None,
            physical_next:None,
            visibility:None,
        }
    }
    pub fn bind_sprites(&mut self, encoded:&[u8], directory:&[u8], sim:&sims_sim::Simulation)->Result<()>{
        ensure!(self.draw_source.is_none(),"object sprite source already bound");
        self.draw_source=Some(object_draw::Source::import(encoded,directory,sim)?);
        if self.stage==Stage::Blocked(Pending::ObjectDraw){self.stage=Stage::ObjectLoop;}
        self.validate()
    }
    pub fn bind_frames(&mut self,bytes:&[u8])->Result<()>{self.draw_source.as_mut().context("object sprite source absent")?.bind_frames(bytes)?;self.validate()}
    pub fn bind_objects(&mut self, encoded: &[u8]) -> Result<()> {
        ensure!(
            self.objects.is_none(),
            "ordinary object directory already bound"
        );
        self.objects = Some(ordinary_objects::Source::import(encoded)?);
        // Historical frontier snapshots stopped before the first registry
        // lookup. Supply immutable identities without replaying any effects.
        if self.stage == Stage::Blocked(Pending::Objects) {
            ensure!(
                self.head.is_some()
                    && self
                        .placement
                        .as_ref()
                        .is_some_and(PlacementUpdate::complete),
                "ordinary object frontier differs"
            );
            self.stage = Stage::PlaceReturned;
        }
        self.validate()
    }
    pub fn bind_physical(&mut self,encoded:&[u8])->Result<()> {
        self.objects.as_mut().context("ordinary directory absent")?.bind_physical(encoded)?;
        if self.stage==Stage::Blocked(Pending::ObjectNext){self.stage=Stage::ObjectNextCall;}
        self.validate()
    }
    fn current_id(&self)->Result<i16>{self.selected_id.or(self.head).context("selected object identity absent")}
    pub fn visibility_write(&self)->Option<(i16,i16,i16)>{let v=self.visibility.as_ref()?;match self.stage {
        Stage::HideSaved=>Some((v.id,v.prior,1)),Stage::VisibilityRestoreStore=>Some((v.id,1,sims_sim::lot_presenter::object_cutaway::restored(v.prior,true,1))),_=>None}}
    pub fn commit_visibility_write(&mut self,effect:(i16,i16,i16))->Result<()>{
        ensure!(self.visibility_write()==Some(effect),"visibility store acknowledgement differs");
        self.stage=match self.stage{Stage::HideSaved=>Stage::HideWritten,Stage::VisibilityRestoreStore=>Stage::VisibilityRestored,_=>anyhow::bail!("visibility store not pending")};self.validate()
    }
    pub fn bind(&mut self, encoded: &[u8]) -> Result<()> {
        ensure!(
            self.source.is_none() && self.stage == Stage::Entry,
            "ordinary source already executing"
        );
        self.source = Some(Source::import(encoded)?);
        self.validate()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.second_return.is_none_or(|v|v),"ordinary second-half return owner is invalid");
        ensure!(
            self.request.kind == tile_scan::Kind::Ordinary
                && self.request.count > 0
                && self.index >= 0
                && self.index <= self.request.count,
            "ordinary row owner/count differs"
        );
        if let Some(s)=&self.draw_source{s.validate()?;}
        ensure!(self.draw_cursor.is_none()||self.draw_source.is_some(),"draw cursor lost its source");
        if let Some(s) = &self.objects {
            s.validate()?;
        }
        if let Some(s) = &self.source {
            ensure!(
                s.identity == SOURCE && s.view.len() == 0x520 && s.flags.len() == 14,
                "ordinary controls differ"
            );
        }
        if let Some(c)=&self.split_cursor{c.validate()?;}
        ensure!(!matches!(self.stage,Stage::LightEntry|Stage::Splitting|Stage::LightMapped|Stage::LightResult)||self.split_cursor.is_some()||self.normal_light_primary.is_some(),"ordinary split child absent");
        if let Some(l) = &self.light_cursor {
            l.validate().map_err(anyhow::Error::msg)?;
        }
        if let Some(w)=&self.expanded{ensure!(w.result.0.len()==44&&w.copied<=11,"ordinary expanded-wall owner differs");}
        if let Some(n)=&self.physical_next{n.validate().map_err(anyhow::Error::msg)?;}
        if let Some(v)=&self.visibility{ensure!(v.id>0&&self.current_id()?==v.id&&self.draw_source.as_ref().is_some_and(|s|s.id==v.id),"visibility override lost selected owner");}
        ensure!(!matches!(self.stage,Stage::HideRead|Stage::HideRemembered|Stage::HideSaved|Stage::HideWritten|Stage::VisibilityRestoreRead|Stage::VisibilityRestoreStore)||self.visibility.is_some(),"temporary visibility continuation absent");
        Ok(())
    }
    pub fn expanded_wall_copy_progress(&self)->Option<usize>{self.expanded.as_ref().map(|w|w.copied)}
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    pub fn pending(&self) -> Option<Pending> {
        if self.source.is_none() {
            Some(Pending::Source)
        } else if self.visibility_write().is_some(){
            Some(Pending::ObjectVisibility)
        } else if self.stage==Stage::Drawing&&self.draw_cursor.as_ref().is_some_and(|c|c.complete()){
            Some(Pending::ObjectDrawReturn)
        } else if self.draw_cursor.as_ref().is_some_and(|c|c.pc()==0x437b50){
            Some(Pending::SpriteRaster)
        } else if self.draw_cursor.as_ref().is_some_and(|c|c.pc()==0x437a0b) {
            Some(Pending::SpriteFrame)
        } else if self.draw_cursor.as_ref().is_some_and(|c|matches!(c.pc(),0x437ce8|0x437d07)){Some(Pending::AuxiliaryObjectDraw)
        } else if self.draw_cursor.as_ref().is_some_and(|c|c.pc()==0x437cbd){
            Some(Pending::ContainedSlotDraw)
        } else if let Stage::Blocked(p) = self.stage {
            Some(p)
        } else {
            None
        }
    }
    pub(super) fn shadow_active(&self)->bool{self.stage==Stage::Shadow}
    pub(super) fn resume_shadow(&mut self){if self.stage==Stage::Blocked(Pending::ObjectShadow){self.stage=Stage::PoolDone;}}
    pub(super) fn compare_shadow(&self,src:&shadow::Source,q:&serde_json::Value,ctx:&Frame,data:&sims_sim::lot_presenter::terrain_raster::Data)->Result<()>{self.shadow_job.as_ref().context("shadow child absent")?.compare(src,q,ctx,data)}
    pub(super) fn compare_raster(&self,source:&object_draw::raster::Source,q:&serde_json::Value,data:&sims_sim::lot_presenter::terrain_raster::Data)->Result<()>{self.draw_cursor.as_ref().context("draw child absent")?.compare_raster(source,q,data)}
    pub(super) fn raster_store_progress(&self)->Option<(usize,usize)>{self.draw_cursor.as_ref()?.raster_store_progress()}
    pub fn advance(
        &mut self,
        sim: &sims_sim::Simulation,
        rotation: u8,
        split_source: Option<&split::Source>,
        context: &mut Frame,
        placement: Option<&mut placement::Source>,
        floor: Option<&floor::Source>,
        mut classify: Option<&mut classification::State>,
        clip:[i16;4],
        catalog:Option<&object_draw::Catalog>,
        raster:Option<&mut object_draw::raster::Source>,
        shadow:Option<&shadow::Source>,
        data:Option<&mut sims_sim::lot_presenter::terrain_raster::Data>,
    ) -> Result<bool> {
        self.validate()?;
        let Some(source) = &self.source else {
            return Ok(false);
        };
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("ordinary architecture absent")?;
        let cell = |p: [i8; 3]| -> Result<usize> {
            ensure!(
                p[0] >= 0 && p[1] >= 0 && (p[0] as usize) < lot.size && (p[1] as usize) < lot.size,
                "ordinary point outside grid"
            );
            Ok(p[0] as usize * lot.size + p[1] as usize)
        };
        self.stage = match self.stage {
            Stage::Entry => Stage::UiCall,
            Stage::UiCall => Stage::UiReturned,
            Stage::UiReturned => Stage::Copied,
            Stage::Copied => Stage::Loop,
            Stage::Loop => Stage::RotateCall,
            Stage::RotateCall => {
                let [x, y, z] = self.point;
                self.rotated = Some(match 4u8.wrapping_sub(rotation) & 3 {
                    1 => [63 - y, x, z],
                    2 => [63 - x, 63 - y, z],
                    3 => [y, 63 - x, z],
                    _ => [x, y, z],
                });
                Stage::Rotated
            }
            Stage::Rotated => {
                if source.view[0x50] != 0 && source.editor == 0 {
                    Stage::Blocked(Pending::Editor)
                } else {
                    Stage::TerrainCall
                }
            }
            Stage::TerrainCall => {
                self.terrain = Some(
                    sims_sim::lot_presenter::room_masks::terrain_corners(
                        lot.size,
                        &lot.heights,
                        self.point,
                        rotation,
                    )
                    .map_err(anyhow::Error::msg)?,
                );
                Stage::TerrainReturned
            }
            Stage::TerrainReturned => {
                let p = self.rotated.context("ordinary rotated point absent")?;
                let f = lot
                    .floor(i32::from(p[2]))
                    .context("ordinary floor absent")?;
                let i = cell(p)?;
                self.covering = Some(f.source.coverings[i]);
                if f.source.walls[i].segments != 0 {
                    if rotation==0 {Stage::ExpandedCall}else{Stage::Blocked(Pending::ExpandedWalls)}
                } else {
                    Stage::HeadCall
                }
            }
            Stage::ExpandedCall=>{
                ensure!(rotation==0,"ordinary expanded rotation requires its original child");
                let p=self.rotated.context("expanded wall physical point absent")?;
                let floor=lot.floor(i32::from(p[2])).context("expanded wall floor absent")?;
                let result=sims_sim::native_wall_io::get_cells(lot.size,lot.active_size.unwrap_or(lot.size),&floor.source.walls,sims_sim::surface_remap::Coord{x:p[0]as u8,y:p[1]as u8,level:p[2]as u8}).map_err(|e|anyhow::anyhow!("expanded wall service: {e:?}"))?;
                self.expanded=Some(ExpandedQuery{result,copied:0});Stage::ExpandedReturned
            }
            Stage::ExpandedReturned=>Stage::ExpandedCopy,
            Stage::ExpandedCopy=>{
                let w=self.expanded.as_mut().context("expanded wall return absent")?;
                ensure!(w.copied<11,"expanded wall copy already complete");w.copied+=1;
                if w.copied==11{Stage::HeadCall}else{Stage::ExpandedCopy}
            }
            Stage::HeadCall => {
                let p = self.rotated.context("ordinary rotated point absent")?;
                let links = &sim
                    .native_lot
                    .as_ref()
                    .context("ordinary physical links absent")?
                    .placement
                    .links;
                ensure!(
                    links.size == lot.size,
                    "ordinary physical grid size differs"
                );
                let head = *links
                    .heads
                    .get(usize::try_from(i32::from(p[2]) - 1)?)
                    .and_then(|h| h.get(cell(p).ok()?))
                    .context("ordinary physical head owner absent")?;
                // Preserve every nonzero head, including an unregistered ID.
                // Registry failure is an object child, never an empty tile.
                self.head = (head != 0).then_some(head);
                Stage::HeadReturned
            }
            Stage::HeadReturned => Stage::FineCall,
            Stage::FineCall => {
                self.fine = Some([
                    (i32::from(self.point[1]) << 4) | 8,
                    (i32::from(self.point[0]) << 4) | 8,
                ]);
                Stage::FineReturned
            }
            Stage::FineReturned => Stage::LightCall,
            Stage::LightCall => {
                let level = i32::from(self.point[2]);
                let f = lot.floor(level).context("ordinary lighting floor absent")?;
                let env = &sim
                    .room_environment
                    .as_ref()
                    .context("ordinary lighting environment absent")?
                    .manager;
                match map_sample(
                    self.fine.context("ordinary fine point absent")?,
                    level,
                    rotation,
                    lot.size,
                    &f.rooms.primary,
                    env.light_bytes
                        .get((level - 1) as usize)
                        .context("ordinary room light bytes absent")?,
                )
                .map_err(anyhow::Error::msg)?
                {
                    Sample::Value(v) => {
                        self.light = Some(v);
                        if split_source.is_some(){
                            let p=self.rotated.context("ordinary mapped light point absent")?;
                            self.normal_light_primary=Some(f.rooms.primary[cell(p)?]);
                            Stage::LightEntry
                        }else{Stage::LightReturned}
                    }
                    Sample::Split { .. } => {
                        if split_source.is_some() && level==1 {
                            self.split_cursor=Some(split::Cursor::new());
                            Stage::LightEntry
                        }else{Stage::Blocked(Pending::SplitLight)}
                    },
                }
            }
            Stage::LightEntry => if self.split_cursor.is_some(){Stage::Splitting}else{Stage::LightMapped},
            Stage::LightMapped=>Stage::LightResult,
            Stage::LightResult=>Stage::LightReturned,
            Stage::Splitting => {
                let c=self.split_cursor.as_mut().context("ordinary split cursor absent")?;
                c.advance(split_source.context("ordinary split source absent")?,sim,self.fine.context("ordinary split point absent")?,rotation)?;
                if c.complete(){self.light=Some(c.result().context("ordinary split result absent")?);Stage::LightReturned}else{Stage::Splitting}
            }
            Stage::LightReturned => {self.split_cursor=None;self.normal_light_primary=None;Stage::PublishCall},
            Stage::PublishCall => {
                self.light_cursor = Some(LightUpdate::new(
                    self.light.context("ordinary light absent")?,
                ));
                Stage::Publishing
            }
            Stage::Publishing => {
                let l = self
                    .light_cursor
                    .as_mut()
                    .context("ordinary light child absent")?;
                ensure!(
                    context.advance_light(l).map_err(anyhow::Error::msg)?,
                    "ordinary light child stalled"
                );
                if l.complete() {
                    Stage::PublishReturned
                } else {
                    Stage::Publishing
                }
            }
            Stage::PublishReturned => Stage::PlaceFineCall,
            Stage::PlaceFineCall => {
                let p = self
                    .rotated
                    .context("ordinary rotated placement point absent")?;
                self.placement_fine =
                    Some([(i32::from(p[1]) << 4) | 8, (i32::from(p[0]) << 4) | 8]);
                Stage::PlaceFineReturned
            }
            Stage::PlaceFineReturned => Stage::PlaceCall,
            Stage::PlaceCall => {
                self.placement = Some(PlacementUpdate::new(
                    self.placement_fine
                        .context("ordinary placement point absent")?,
                    i32::from(self.point[2]),
                    -1,
                    rotation,
                ));
                Stage::Placing
            }
            Stage::Placing => {
                let p = self
                    .placement
                    .as_mut()
                    .context("ordinary placement absent")?;
                let Some(source) = placement else {
                    return Ok(false);
                };
                if !source.advance(p, sim, context)? {
                    return Ok(false);
                }
                if p.complete() {
                    Stage::PlaceReturned
                } else {
                    Stage::Placing
                }
            }
            Stage::PlaceReturned => {
                if let Some(id) = self.head {
                    if let Some(objects) = &self.objects {
                        if let Some(pointer) = objects.lookup(id, sim)? {
                            self.selected = Some(pointer);
                            Stage::LookupSelected
                        } else {
                            Stage::Blocked(Pending::ObjectSource)
                        }
                    } else {
                        Stage::Blocked(Pending::Objects)
                    }
                } else {
                    Stage::ObjectGate
                }
            }
            Stage::LookupSelected => Stage::LookupStored,
            Stage::LookupStored => {
                if self.selected == Some(0) {
                    Stage::Blocked(Pending::RegistryRepair)
                } else {
                    Stage::ObjectGate
                }
            }
            Stage::ObjectGate => Stage::QuadCall,
            Stage::QuadCall => {
                self.quad = Some(
                    floor
                        .context("ordinary descriptor owner absent")?
                        .geometry_descriptor(self.point, rotation)?,
                );
                Stage::QuadReturned
            }
            Stage::QuadReturned => {
                if i32::from(self.point[2]) > 1 {
                    Stage::Blocked(Pending::UpperFloor)
                } else {
                    Stage::AfterFloor
                }
            }
            Stage::AfterFloor => {
                if source.terrain_overlay != 0 {
                    Stage::Blocked(Pending::TerrainOverlay)
                } else {
                    Stage::OverlayDone
                }
            }
            Stage::OverlayDone => {
                if self.point[2] == 1 {
                    Stage::PoolCall
                } else {
                    Stage::PoolDone
                }
            }
            Stage::PoolCall => Stage::PoolReturned,
            Stage::PoolReturned => {
                if source.pool {
                    self.pool_value = Some(
                        lot.pool[cell(self.rotated.context("ordinary rotated point absent")?)?],
                    );
                    if self.pool_value.is_some_and(|v| v <= 16) {
                        Stage::Blocked(Pending::Pool)
                    } else {
                        Stage::PoolDone
                    }
                } else {
                    Stage::PoolDone
                }
            }
            // 4ae9e4 is reached even for an empty physical head. Keep that
            // native boundary instead of jumping directly to the row tail.
            Stage::PoolDone => {
                if self.selected.is_some_and(|p|p!=0){
                    let id=self.head.context("shadow object head absent")?;
                    if shadow.is_some(){self.shadow_job=Some(shadow::Job::new(id,self.point,rotation));Stage::Shadow}else if !self.objects.as_ref().context("shadow directory owner absent")?.shadow_absent(id,sim)?{Stage::Blocked(Pending::ObjectShadow)}else{Stage::Objects}
                }else{Stage::Objects}
            },
            Stage::Shadow=>{let j=self.shadow_job.as_mut().context("shadow child absent")?;if j.complete(){Stage::Objects}else{let(Some(src),Some(p),Some(data))=(shadow,placement,data)else{return Ok(false);};if !j.advance(src,p,sim,context,data,clip)?{return Ok(false);}Stage::Shadow}},
            Stage::Objects => {
                if self.head.is_some() && source.flags[6] != 0 {
                    Stage::ObjectLoop
                } else {
                    Stage::Tail
                }
            }
            Stage::ObjectLoop => {
                if self.selected==Some(0){self.stage=Stage::Tail;return Ok(true);}
                let id=self.current_id()?;
                //4aea06..4aea0c skips dynamic objects in this pass before any
                //drawable-group lookup, placement, rectangle clear, or draw.
                let state=sim.vm.world.refresh.as_ref().and_then(|r|r.draw.get(&id)).context("object draw state unobserved")?;
                if state.dynamic{self.stage=Stage::ObjectNextCall;return Ok(true);}
                if let Some(catalog)=catalog{
                    if self.draw_source.as_ref().is_none_or(|s|!s.owns(id,self.selected.unwrap_or(0),sim)){
                        let Some(next)=catalog.select(id,self.selected.context("selected object absent")?,sim)else{self.stage=Stage::Blocked(Pending::ObjectSource);return Ok(true);};
                        self.draw_source=Some(next);self.draw_cursor=None;
                    }
                }
                let Some(draw)=&self.draw_source else{self.stage=Stage::Blocked(if catalog.is_some(){Pending::ObjectSource}else{Pending::ObjectDraw});return Ok(true);};
                if !draw.owns(id,self.selected.context("object selected identity absent")?,sim){return Ok(false);}
                let [x,y,_]=self.rotated.context("object rotated point absent")?.map(i32::from);
                let [a,b,c,d]=draw.bounds;
                if x<a.wrapping_add(1)||x>b.wrapping_sub(1)||y<c.wrapping_add(1)||y>d.wrapping_sub(1){return Ok(false);}
                Stage::ObjectPlaceCall
            }
            Stage::ObjectPlaceCall=>{
                let p=sim.vm.world.positions.get(&self.current_id()?).context("object physical position absent")?;
                self.placement=Some(PlacementUpdate::new([p.x,p.y],self.request.level,-1,rotation));
                Stage::ObjectPlacing
            }
            Stage::ObjectPlacing=>{
                let p=self.placement.as_mut().context("object placement child absent")?;
                let Some(s)=placement else{return Ok(false);};if !s.advance(p,sim,context)?{return Ok(false);}
                if p.complete(){Stage::ObjectPlaceReturned}else{Stage::ObjectPlacing}
            }
            Stage::ObjectPlaceReturned=>Stage::ObjectClearCall,
            Stage::ObjectClearCall=>{
                let Some(c)=classify.as_deref_mut()else{return Ok(false);};
                c.clear_cached_rectangle(self.current_id()?,1)?;
                Stage::ObjectClearReturned
            }
            Stage::ObjectClearReturned=>Stage::ObjectUiCall,
            Stage::ObjectUiCall=>Stage::ObjectUiReturned,
            Stage::ObjectUiReturned=>{
                if self.draw_source.as_ref().context("object drawing source absent")?.hidden(self.rotated.context("object rotated point absent")?,rotation,sim)?{
                    let id=self.current_id()?;let prior=*sim.vm.world.objects.get(&id).context("visibility owner absent")?.data.get(&34).context("visibility word absent")?;
                    self.visibility=Some(VisibilityOverride{id,prior});Stage::HideRead
                }else{Stage::VisibilityMerged}
            }
            Stage::VisibilityMerged=>Stage::ObjectDrawCall,
            Stage::HideRead=>Stage::HideRemembered,
            Stage::HideRemembered=>Stage::HideSaved,
            Stage::HideSaved=>return Ok(false),
            Stage::HideWritten=>Stage::ObjectDrawCall,
            Stage::ObjectDrawCall=>{self.draw_cursor=Some(self.draw_source.as_ref().context("object drawing source absent")?.new_draw());Stage::Drawing}
            Stage::Drawing=>{
                if self.draw_cursor.as_ref().is_some_and(|d|d.complete()){self.stage=Stage::ObjectDrawReturned;return Ok(true);}
                let Some(c)=classify.as_deref_mut()else{return Ok(false);};
                if !self.draw_cursor.as_mut().context("object draw cursor absent")?.advance(self.draw_source.as_ref().context("object drawing source absent")?,sim,context,c,clip,raster.zip(data),catalog,self.objects.as_ref(),placement,rotation)?{return Ok(false);}
                Stage::Drawing
            },
            Stage::ObjectDrawReturned=>if self.visibility.is_some(){Stage::VisibilityRestoreRead}else{Stage::VisibilityRestored},
            Stage::VisibilityRestoreRead=>Stage::VisibilityRestoreStore,
            Stage::VisibilityRestoreStore=>return Ok(false),
            Stage::VisibilityRestored=>Stage::ObjectFlagsCall,
            Stage::ObjectFlagsCall=>{context.set_render_byte(0x37,0).map_err(anyhow::Error::msg)?;Stage::ObjectFlagsReturned}
            Stage::ObjectFlagsReturned=>Stage::ObjectNextCall,
            Stage::ObjectNextCall=>{
                let id=self.current_id()?;
                let next=*sim.native_lot.as_ref().context("ordinary physical owner absent")?.placement.links.next.get(&id).context("ordinary physical next unknown")?;
                if self.objects.as_ref().and_then(|s|s.physical(id)).is_some(){
                    let source=self.objects.as_ref().context("ordinary directory absent")?;
                    self.next_id=(next>0).then_some(next);
                    self.physical_next=Some(sims_sim::lot_presenter::physical_next::Next::new(next,(next>0).then_some(source.range())).map_err(anyhow::Error::msg)?);
                    self.stage=Stage::ObjectNextExecuting;return Ok(true);
                }
                if next!=0{Stage::Blocked(Pending::ObjectNext)}else{self.next_selected=Some(0);Stage::ObjectNextReturned}
            }
            Stage::ObjectNextExecuting=>{
                let source=self.objects.as_ref().context("ordinary directory absent")?;
                let child=self.physical_next.as_mut().context("physical-next child absent")?;
                if !child.advance(|id|source.lookup(id,sim).map_err(|e|format!("{e:#}"))).map_err(anyhow::Error::msg)?{return Ok(false);}
                if child.complete(){self.next_selected=child.result();Stage::ObjectNextReturned}else{Stage::ObjectNextExecuting}
            }
            Stage::ObjectNextReturned=>{self.selected=self.next_selected;self.visibility=None;if self.physical_next.is_some(){self.selected_id=self.next_id;}Stage::ObjectLoop}

            Stage::Tail => {
                self.index = self.index.wrapping_add(1);
                self.point[0] = self.point[0].wrapping_add(self.request.step[0]);
                self.point[1] = self.point[1].wrapping_add(self.request.step[1]);
                self.screen[0] = self.screen[0].wrapping_add(word(&source.view, 0x58) as i32);
                if self.index < self.request.count {
                    self.split_cursor=None;
                    self.rotated = None;
                    self.terrain = None;
                    self.quad = None;
                    self.head = None;
                    self.selected = None;
                    self.selected_id=None;
                    self.next_id=None;
                    self.physical_next=None;
                    self.visibility=None;
                    self.covering = None;
                    self.fine = None;
                    self.placement_fine = None;
                    self.light = None;
                    self.light_cursor = None;
                    self.placement = None;
                    self.expanded = None;
                    self.pool_value = None;
                    Stage::Loop
                } else {
                    Stage::Epilogue
                }
            }
            Stage::Epilogue => Stage::Returned,
            Stage::Returned | Stage::Blocked(_) => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
    pub fn compare_auxiliary(&self,q:&serde_json::Value,catalog:&object_draw::Catalog)->Result<()>{if self.stage==Stage::Drawing{self.draw_cursor.as_ref().context("draw absent")?.compare_auxiliary(q,self.draw_source.as_ref().context("draw source absent")?,catalog)?;}Ok(())}
    pub fn matches_observation(&self,q:&serde_json::Value)->Result<bool>{
        if let Some(v)=q.get("visibility_word"){
            let pointer=v["pointer"].as_u64().context("visibility observation pointer absent")?as u32;
            let raw=decode(&q["ordinary_frame"]["raw"])?;
            if self.selected!=Some(pointer)||raw.get(0x220..0x223)!=Some(self.point.map(|v|v as u8).as_slice()){
                return Ok(false);
            }
        }
        if self.stage==Stage::Drawing{self.draw_cursor.as_ref().context("ordinary draw absent")?.matches_observation(q)}else{Ok(true)}
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        use self::Stage::*;
        let at = match self.stage {
            Entry => 0x4ad8ac,
            UiCall => 0x4ad8cd,
            UiReturned => 0x4ad8d2,
            Copied => 0x4ad8f1,
            Loop => 0x4ad916,
            RotateCall => 0x4ad950,
            Rotated => 0x4ad955,
            TerrainCall => 0x4ad9b3,
            TerrainReturned => 0x4ad9b8,
            HeadCall => 0x4ada4e,
            HeadReturned => 0x4ada53,
            FineCall => 0x4ada8b,
            FineReturned => 0x4ada90,
            LightEntry => 0x467482,
            LightMapped => 0x4674e4,
            LightResult => 0x46767f,
            Splitting => return self.split_cursor.as_ref().is_some_and(|c|c.at_native_cut(pc)),
            LightCall => 0x4ada9d,
            LightReturned => 0x4adaa2,
            PublishCall => 0x4adaa9,
            PublishReturned => 0x4adaae,
            PlaceFineCall => 0x4adac0,
            PlaceFineReturned => 0x4adac5,
            PlaceCall => 0x4adac9,
            PlaceReturned => 0x4adace,
            LookupSelected => 0x4adb03,
            LookupStored => 0x4adb08,
            ObjectGate => 0x4adb25,
            Objects => 0x4ae9e4,
            ObjectLoop => 0x4ae9fe,
            ObjectPlaceCall=>0x4aea7a,
            ObjectPlaceReturned=>0x4aea7f,
            ObjectClearCall=>0x4aea83,
            ObjectClearReturned=>0x4aea88,
            ObjectUiCall=>0x4aeaa8,
            ObjectUiReturned=>0x4aeaad,
            ObjectDrawCall=>0x4aebb8,
            Drawing=>return self.draw_cursor.as_ref().is_some_and(|c|c.pc()==pc),
            ObjectDrawReturned=>0x4aebbb,
            ObjectFlagsCall=>0x4aebd2,
            ObjectFlagsReturned=>0x4aebd7,
            ObjectNextCall=>0x4aebde,
            ObjectNextReturned=>0x4aebe3,
            ObjectNextExecuting=>self.physical_next.as_ref().map_or(0,|n|n.pc()),
            VisibilityMerged=>0x4aeba3,
            HideRead=>0x4aeb88,HideRemembered=>0x4aeb9a,HideSaved=>0x4aeb9e,HideWritten=>0x4aeba3,
            VisibilityRestoreRead=>0x4aebc1,VisibilityRestoreStore=>0x4aebc6,VisibilityRestored=>0x4aebcd,
            ObjectPlacing=>return false,
            QuadCall => 0x4adb48,
            QuadReturned => 0x4adb4d,
            AfterFloor => 0x4ae5fd,
            OverlayDone => 0x4ae63b,
            PoolCall => 0x4ae64b,
            PoolReturned => 0x4ae650,
            PoolDone => 0x4ae83e,
            Shadow=>return self.shadow_job.as_ref().is_some_and(|j|j.pc()==pc),
            Tail => 0x4aebed,
            Epilogue => 0x4aec23,
            Returned => self.return_pc(),
            ExpandedCall=>0x4ada1c,
            ExpandedReturned=>0x4ada21,
            ExpandedCopy=>0x4ada2c,
            Publishing | Placing | Blocked(_) => return false,
        };
        pc == at
    }
    pub fn compare(&self, q: &serde_json::Value, context: &Frame) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("ordinary PC absent")? as u32;
        ensure!(
            q["tick"] == 89081 && self.at_native_cut(pc),
            "ordinary boundary differs at{pc:x}"
        );
        let native = decode(&q["context"]["raw"])?;
        ensure!(
            context
                .state()
                .words()
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
                == native,
            "ordinary shared context differs at{pc:x}"
        );
        if self.stage==Stage::LightEntry {
            let input=&q["split_light"]["input"];
            let raw=decode(&input["physical"])?;
            ensure!([word(&raw,0)as i32,word(&raw,4)as i32]==self.fine.context("ordinary split point absent")?&&input["level"]==self.point[2]&&q["split_light"]["caller"]==0x4adaa2u32,"ordinary split entry ABI differs");
        }
        if self.stage==Stage::LightMapped{ensure!(decode(&q["split_light"]["primary_room"]["raw"])?==self.normal_light_primary.context("ordinary room-map result absent")?.to_le_bytes(),"ordinary light primary room differs");}
        if self.stage==Stage::Splitting{self.split_cursor.as_ref().context("ordinary split cursor absent")?.compare(q)?;}
        if self.stage==Stage::PublishCall&&q["split_light"].is_object(){ensure!(word(&decode(&q["split_light"]["caller_float_bits"])?,0)==self.light.context("ordinary float reply absent")?,"ordinary split caller float spill differs");}
        if q["visibility_inputs"].is_object(){self.draw_source.as_ref().context("visibility input source absent")?.compare_visibility_inputs(q)?;}
        if let Some(v)=&self.visibility{
            let native=&q["visibility_word"];
            if native.is_object(){
                let before_store=matches!(self.stage,Stage::HideRead|Stage::HideRemembered|Stage::HideSaved);
                let restored=matches!(self.stage,Stage::VisibilityRestored|Stage::ObjectFlagsCall|Stage::ObjectFlagsReturned|Stage::ObjectNextCall|Stage::ObjectNextExecuting|Stage::ObjectNextReturned);
                let word=if before_store{v.prior}else if restored{sims_sim::lot_presenter::object_cutaway::restored(v.prior,true,1)}else{1};
                //SETNZ has not written the saved truth byte at either of
                //these cuts. It is neither a zero input nor live state yet.
                let truth_written=!matches!(self.stage,Stage::HideRead|Stage::HideRemembered);
                ensure!(native["id"]==v.id&&native["word"]==word
                    &&(!truth_written||native["previous_truth"]==u8::from(v.prior!=0))
                    &&native["restore_flag"]==u8::from(self.stage!=Stage::HideRead),"temporary visibility word/lifetime differs");
            }
        }
        if self.stage==Stage::Shadow{return Ok(());}
        if self.stage==Stage::Drawing{
            let c=self.draw_cursor.as_ref().context("object draw cursor absent")?;c.compare(q,self.draw_source.as_ref().context("object drawing source absent")?)?;
            if c.complete(){let f=&q["ordinary_frame"];let raw=decode(&f["raw"])?;ensure!(c.result()==Some(std::array::from_fn(|i|word(&raw,0x64+4*i)as i32)),"returned object rectangle differs");}
            return Ok(());
        }
        // RET has ended the row frame's lifetime. Its stale bytes remain in
        // the raw witness, but are not compared as surviving row state.
        if self.stage == Stage::Returned {
            return Ok(());
        }
        let f = &q["ordinary_frame"];
        let bp = f["bp"].as_u64().context("ordinary BP absent")? as u32;
        let raw = decode(&f["raw"])?;
        ensure!(
            f["address"] == bp - 0x230 && raw.len() == 0x290,
            "ordinary frame extent differs"
        );
        if let Some(w)=&self.expanded{
            if matches!(self.stage,Stage::ExpandedReturned|Stage::ExpandedCopy){
                for(n,b)in w.result.0.iter().enumerate(){ensure!(raw[8+n]&b.known==b.value&b.known,"expanded wall getter differs at byte{n}");}
                if self.stage==Stage::ExpandedReturned{ensure!(q["registers"]["eax"]==bp-0x228,"expanded wall return address differs");}
            }
            for(n,b)in w.result.0[..w.copied*4].iter().enumerate(){ensure!(raw[0x114+n]&b.known==b.value&b.known,"expanded wall local copy differs at byte{n}");}
        }
        if self.selected.is_some() && self.selected!=Some(0)
            && matches!(
                self.stage,
                Stage::LookupSelected
                    | Stage::LookupStored
                    | Stage::ObjectGate
                    | Stage::Objects
                    | Stage::ObjectLoop
            )
        {
            let objects=self.objects.as_ref().context("ordinary object source absent")?;
            if let Some(id)=self.selected_id {objects.compare_cursor(q,id,self.selected.unwrap())?;}
            else {objects.compare(q,self.head.context("ordinary object head absent")?,self.selected.unwrap())?;}
        }
        if self.selected.is_some()
            && matches!(
                self.stage,
                Stage::LookupStored | Stage::ObjectGate | Stage::Objects | Stage::ObjectLoop
            )
        {
            ensure!(
                word(&raw, 0x1d8) == self.selected.unwrap(),
                "ordinary selected local differs"
            );
        }
        if self.stage==Stage::ObjectNextReturned{ensure!(q["registers"]["eax"]==self.next_selected.context("next pointer absent")?,"physical next return differs");}
        if matches!(self.stage,Stage::ObjectNextExecuting|Stage::ObjectNextReturned) {
            if let Some(child)=&self.physical_next {
                self.objects.as_ref().context("ordinary directory absent")?.compare_physical(q,self.current_id()?,self.selected.context("ordinary selected pointer absent")?,child)?;
            }
        }
        if self.stage == Stage::UiReturned {
            ensure!(
                q["registers"]["eax"]
                    == self.source.as_ref().context("ordinary controls absent")?.ui,
                "ordinary UI return differs"
            );
        }
        if self.stage != Stage::Entry
            && self.stage != Stage::UiCall
            && self.stage != Stage::UiReturned
        {
            ensure!(
                raw[0x220..0x223] == self.point.map(|v| v as u8),
                "ordinary local point differs at{pc:x}"
            );
        }
        if let Some(p) = self.rotated {
            ensure!(
                raw[0x21c..0x21f] == p.map(|v| v as u8),
                "ordinary rotated point differs at{pc:x}"
            );
        }
        if let Some(t) = self.terrain {
            ensure!(
                raw[0x1e4..0x1e8] == t,
                "ordinary terrain corners differ at{pc:x}"
            );
        }
        if let Some(p) = self.fine {
            ensure!(
                [word(&raw, 0x104) as i32, word(&raw, 0x108) as i32] == p,
                "ordinary fine center differs at{pc:x}"
            );
        }
        if let Some(p) = self.placement_fine {
            ensure!(
                [word(&raw, 0x10c) as i32, word(&raw, 0x110) as i32] == p,
                "ordinary rotated placement center differs"
            );
        }
        if self.stage == Stage::HeadReturned {
            ensure!(
                q["registers"]["eax"]
                    .as_u64()
                    .context("ordinary head return absent")? as u16
                    == self.head.unwrap_or(0) as u16,
                "ordinary head return differs"
            );
        }
        if let Some(qd) = self.quad {
            ensure!(
                raw[0x208..0x20c] == qd,
                "ordinary descriptor differs at{pc:x}"
            );
        }
        if let Some(pool) = self.pool_value {
            ensure!(raw[0x1a4] == pool, "ordinary pool value differs");
        }
        if matches!(self.stage, Stage::Epilogue | Stage::Returned) {
            ensure!(
                word(&raw, 0x1a8) as i32 == self.index,
                "ordinary loop counter differs"
            );
        }
        if matches!(self.stage, Stage::Epilogue | Stage::Returned) {
            ensure!(
                [word(&raw, 0x244) as i32, word(&raw, 0x248) as i32] == self.screen,
                "ordinary loop screen differs"
            );
        }
        if self.stage == Stage::Entry {
            let stack = decode(&q["post_base"]["stack"]["raw"])?;
            let scanbp = q["registers"]["ebp"]
                .as_u64()
                .context("ordinary scan BP absent")? as u32;
            let words = std::array::from_fn::<_, 10, _>(|i| word(&stack, 4 * i));
            ensure!(
                words
                    == [
                        self.return_pc(),
                        scanbp - 0x18,
                        self.request.count as u32,
                        scanbp - 0x20,
                        self.request.screen[0] as u32,
                        self.request.screen[1] as u32,
                        q["context"]["pointer"]
                            .as_u64()
                            .context("ordinary context identity absent")?
                            as u32,
                        0x6f0c10,
                        self.request.level as u32,
                        self.request.above
                    ],
                "ordinary row complete entry ABI differs"
            );
        }
        Ok(())
    }
}
